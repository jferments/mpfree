// FXProbeTap.cpp - an insert effect that records what reaches it to a float
// WAV, so it can sit after an AIR effect on the factory MPC and capture that
// effect's output exactly as the host's chain produced it.
//
// It passes audio through untouched (or mutes it, so an unattended run is not
// heard). A "run" command in the work folder's control.txt (see
// ../common/fxprobe.h) starts a capture of a given length:
//
//   captures/<capture>-s<slot>.wav    32-bit float stereo, every frame the host gave us
//   captures/<capture>-s<slot>.txt    report: frames, overruns, peak, block sizes, transport
//
// With timing on (the Timing parameter or "timing 1" in the command) it also
// writes, for the SDK integration work:
//
//   captures/<capture>-s<slot>.timing.tsv     one line per process call: monotonic ns, frames, CPU,
//                                             headed by the audio thread's scheduling policy and priority
//   captures/<capture>-s<slot>.hostcalls.tsv  every dispatcher and setParameter call the host made
//                                             since the plugin was created, in order
//
// Threads: the audio thread copies frames into a lock-free ring and touches
// only atomics; a worker drains the ring to disk and reads the control file.
//
// SPDX-License-Identifier: MIT

#include "mpc_plugin.h"

#include "../../common/fxprobe.h"
#include "../../common/slots.h"

#include <cmath>
#include <memory>

#include <pthread.h>
#include <sched.h>

using namespace fxprobe;

class FXProbeTap : public mpc::Plugin
{
public:
    enum Params { kSlot, kOutput, kTiming, kNumParams };
    enum Action { kNone, kRun, kStop };

    static constexpr size_t kRingFrames = size_t(1) << 19;   // about 12 s at 44.1 kHz
    static constexpr size_t kTickCapacity = size_t(1) << 15;
    static constexpr int kMaxCalls = 8192;
    static constexpr int kHistBuckets = 9;

    static mpc::PluginInfo makeInfo()
    {
        mpc::PluginInfo i;
        i.name = "FXProbe Tap";
        i.vendor = "MPFree";
        i.uniqueId = MPC_FOURCC('M', 'F', 'p', 't');
        i.version = 1000;
        i.isInstrument = false;
        i.numInputs = 2;
        i.numOutputs = 2;
        i.tailSamples = 1;
        return i;
    }

    FXProbeTap() : mpc::Plugin(makeInfo())
    {
        addParam(mpc::choiceParam("slot", "Slot", slotChoices(), 0));
        addParam(mpc::choiceParam("output", "Output", {"Pass", "Mute"}, 0));
        addParam(mpc::choiceParam("timing", "Timing log", {"Off", "On"}, 0));
        ring_.allocate(kRingFrames * 2);
        ticks_.allocate(kTickCapacity);
        autoSlot_ = claimSlot();
        worker_.start([this] { tick(); }, 10);
    }

    ~FXProbeTap() override
    {
        worker_.stop();
        releaseSlot(autoSlot_);
    }

    int slot() const
    {
        int s = getInt(kSlot);
        return s > 0 ? s : autoSlot_;
    }

    void prepare(double sampleRate, int) override { rate_.store((int)sampleRate); }

    void process(mpc::ProcessContext& ctx) override
    {
        const int n = ctx.numFrames;
        const float* inL = ctx.inputs[0];
        const float* inR = ctx.numInputs > 1 ? ctx.inputs[1] : ctx.inputs[0];

        calls_.fetch_add(1, std::memory_order_relaxed);
        int bucket = n <= 32 ? 0 : n <= 64 ? 1 : n <= 96 ? 2 : n <= 128 ? 3 : n <= 192 ? 4 : n <= 256 ? 5 : n <= 512 ? 6 : n <= 1024 ? 7 : 8;
        hist_[bucket].fetch_add(1, std::memory_order_relaxed);
        if (n < minBlock_.load(std::memory_order_relaxed)) minBlock_.store(n, std::memory_order_relaxed);
        if (n > maxBlock_.load(std::memory_order_relaxed)) maxBlock_.store(n, std::memory_order_relaxed);

        const long seq = recSeq_.load(std::memory_order_acquire);
        if (seq != seenSeq_)
        {
            seenSeq_ = seq;
            if (recAction_.load(std::memory_order_relaxed) == kRun)
            {
                remaining_ = recFrames_.load(std::memory_order_relaxed);
                recording_ = remaining_ > 0;
                bpm_ = ctx.transport.tempoValid ? ctx.transport.bpm : 0.0;
                playing_ = ctx.transport.valid && ctx.transport.playing;
                startNs_ = monotonicNs();
                if (!recording_) ack(seq);
            }
            else
            {
                recording_ = false;
                ack(seq);
            }
        }

        const bool timing = timingOn_.load(std::memory_order_relaxed);
        if (timing && recording_)
        {
            if (!schedRead_)
            {
                // Once per capture: these are system calls, cheap but not free.
                int policy = 0;
                sched_param sp{};
                pthread_getschedparam(pthread_self(), &policy, &sp);
                policy_.store(policy, std::memory_order_relaxed);
                priority_.store(sp.sched_priority, std::memory_order_relaxed);
                schedRead_ = true;
            }
            Tick t{monotonicNs(), n, sched_getcpu()};
            if (!ticks_.push(t)) tickDrops_.fetch_add(1, std::memory_order_relaxed);
        }

        if (recording_)
        {
            int64_t take = remaining_ < n ? remaining_ : n;
            float buf[2 * 256];
            for (int64_t i = 0; i < take;)
            {
                int64_t m = take - i < 256 ? take - i : 256;
                for (int64_t k = 0; k < m; ++k)
                {
                    buf[2 * k] = inL[i + k];
                    buf[2 * k + 1] = inR[i + k];
                }
                if (!ring_.push(buf, (size_t)(2 * m))) overrun_.fetch_add(m, std::memory_order_relaxed);
                i += m;
            }
            remaining_ -= take;
            if (remaining_ == 0)
            {
                recording_ = false;
                endNs_ = monotonicNs();
                ack(seenSeq_);
            }
        }

        if (getInt(kOutput) == 1)
            for (int c = 0; c < ctx.numOutputs; ++c)
                std::memset(ctx.outputs[c], 0, sizeof(float) * (size_t)n);
        // else: in place, the input is already the output. With separate
        // buffers the framework does not clear outputs that are not inputs,
        // so copy.
        else
            for (int c = 0; c < ctx.numOutputs && c < ctx.numInputs; ++c)
                if (ctx.outputs[c] != ctx.inputs[c])
                    std::memcpy(ctx.outputs[c], ctx.inputs[c], sizeof(float) * (size_t)n);
    }

    // Called by the logging dispatcher below, from any thread.
    void logCall(int opcode, int index, int64_t value, float opt)
    {
        uint32_t i = nCalls_.fetch_add(1, std::memory_order_relaxed);
        if (i >= (uint32_t)kMaxCalls) return;
        Call& c = callLog_[i];
        c.ns = monotonicNs();
        c.thread = (uint64_t)pthread_self();
        c.opcode = opcode;
        c.index = index;
        c.value = value;
        c.opt = opt;
        c.ready.store(true, std::memory_order_release);
    }

private:
    struct Tick
    {
        int64_t ns = 0;
        int32_t frames = 0;
        int32_t cpu = 0;
    };

    struct Call
    {
        int64_t ns = 0;
        uint64_t thread = 0;
        int32_t opcode = 0;
        int32_t index = 0;
        int64_t value = 0;
        float opt = 0.0f;
        std::atomic<bool> ready{false};
    };

    // ---- audio thread -------------------------------------------------------
    void ack(long seq)
    {
        // Everything written above (bpm, start and end times) is published
        // by this release.
        doneSeq_.store(seq, std::memory_order_release);
    }

    // ---- worker -------------------------------------------------------------
    void tick()
    {
        drain();
        const std::string& f = work_.current();
        if (f != folder_)
        {
            // The folder went (a stick taken out) or a new one came: a
            // capture in progress ends, a command already there is old, and
            // the status is written again.
            if (capturing_) stopCapture();
            folder_ = f;
            watcher_.reset();
            lastStatus_.clear();
        }
        if (folder_.empty()) return;
        if (!watcher_) watcher_.reset(new ControlWatcher(folder_));
        Command c;
        if (watcher_->poll(c) && c.forSlot(slot())) handle(c);
        if (capturing_ && doneSeq_.load(std::memory_order_acquire) == posted_)
        {
            drain();
            if (ring_.used() == 0) finishCapture(false);
        }
        writeStatus();
    }

    void handle(const Command& c)
    {
        error_.clear();
        if (capturing_) stopCapture();
        if (c.action == "stop" || c.action == "load") return;
        if (c.action != "run") { error_ = "unknown action " + c.action; return; }
        if (c.capture.empty()) return;   // a run for the Source only
        if (!safeName(c.capture)) { error_ = "bad capture name"; return; }
        const int rate = rate_.load();
        int64_t frames = (int64_t)std::llround(c.seconds * rate);
        if (frames <= 0) { error_ = "seconds must be more than 0"; return; }
        std::string dir = folder_ + "/captures";
        if (!ensureDir(dir)) { error_ = "cannot make captures/"; return; }
        base_ = fmt("%s/%s-s%d", dir.c_str(), c.capture.c_str(), slot());
        if (!wav_.open(base_ + ".wav", rate)) { error_ = "cannot write " + base_ + ".wav"; return; }
        captureName_ = c.capture;
        captureSeq_ = c.seq;
        requested_ = frames;
        peak_ = 0.0f;
        nonFinite_ = 0;
        overrun_.store(0);
        tickDrops_.store(0);
        callsAtStart_ = calls_.load();
        timingThis_ = c.timing || getInt(kTiming) == 1;
        if (timingThis_)
        {
            timingFile_ = std::fopen((base_ + ".timing.tsv").c_str(), "wb");
            timingHeader_ = false;
        }
        timingOn_.store(timingThis_);
        capturing_ = true;
        state_ = "recording";
        post(kRun, frames);
    }

    void post(int action, int64_t frames)
    {
        recAction_.store(action, std::memory_order_relaxed);
        recFrames_.store(frames, std::memory_order_relaxed);
        posted_ = ++postCount_;
        recSeq_.store(posted_, std::memory_order_release);
    }

    // Stops the capture in progress (a new command arrived first).
    void stopCapture()
    {
        post(kStop, 0);
        for (int i = 0; i < 100 && doneSeq_.load(std::memory_order_acquire) != posted_; ++i)
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
        drain();
        finishCapture(true);
    }

    void drain()
    {
        float buf[2 * 4096];
        for (;;)
        {
            size_t got = ring_.pop(buf, 2 * 4096);
            if (got == 0) break;
            for (size_t i = 0; i < got; ++i)
            {
                float v = buf[i];
                if (!std::isfinite(v)) ++nonFinite_;
                else if (std::fabs(v) > peak_) peak_ = std::fabs(v);
            }
            if (wav_.isOpen() && !wav_.write(buf, got / 2)) error_ = "write failed (disk full?)";
        }
        Tick t[256];
        for (;;)
        {
            size_t got = ticks_.pop(t, 256);
            if (got == 0) break;
            if (!timingFile_) continue;
            if (!timingHeader_)
            {
                int pol = policy_.load();
                const char* pname = pol == SCHED_FIFO ? "SCHED_FIFO" : pol == SCHED_RR ? "SCHED_RR" : pol == SCHED_OTHER ? "SCHED_OTHER" : "other";
                std::fprintf(timingFile_, "# audio thread policy %s (%d) priority %d\nns\tframes\tcpu\n", pname, pol, priority_.load());
                timingHeader_ = true;
            }
            for (size_t i = 0; i < got; ++i)
                std::fprintf(timingFile_, "%lld\t%d\t%d\n", (long long)t[i].ns, t[i].frames, t[i].cpu);
        }
    }

    void finishCapture(bool early)
    {
        capturing_ = false;
        timingOn_.store(false);
        const uint64_t written = wav_.frames();
        wav_.close();
        if (timingFile_) { std::fclose(timingFile_); timingFile_ = nullptr; }
        if (timingThis_) writeHostCalls(base_ + ".hostcalls.tsv");

        std::string r = fmt("fxprobe tap\nversion %d\nslot %d\ncapture %s\nseq %ld\nrate %d\n", kVersion, slot(),
                            captureName_.c_str(), captureSeq_, rate_.load());
        r += fmt("frames_requested %lld\nframes_written %llu\noverrun_frames %lld\nstopped_early %d\n",
                 (long long)requested_, (unsigned long long)written, (long long)overrun_.load(), early ? 1 : 0);
        r += fmt("nonfinite_samples %lld\npeak_dbfs %.2f\n", (long long)nonFinite_,
                 peak_ > 0.0f ? 20.0 * std::log10((double)peak_) : -999.0);
        // Read after the acquire of doneSeq_ in tick(); stale only if stopped early.
        r += fmt("transport_bpm %.3f\ntransport_playing %d\n", bpm_, playing_ ? 1 : 0);
        r += fmt("wall_seconds %.3f\n", endNs_ > startNs_ ? (double)(endNs_ - startNs_) * 1e-9 : 0.0);
        r += fmt("process_calls %lld\nblock_min %d\nblock_max %d\nblock_hist", (long long)(calls_.load() - callsAtStart_),
                 minBlock_.load(), maxBlock_.load());
        static const char* edges[kHistBuckets] = {"32", "64", "96", "128", "192", "256", "512", "1024", "more"};
        for (int i = 0; i < kHistBuckets; ++i) r += fmt(" %s:%u", edges[i], hist_[i].load());
        r += "\n";
        if (timingThis_) r += fmt("timing_drops %lld\n", (long long)tickDrops_.load());
        if (!error_.empty()) r += "error " + error_ + "\n";
        writeAtomically(base_ + ".txt", r);
        state_ = early ? "stopped" : "done";
        doneCapture_ = captureName_;
    }

    void writeHostCalls(const std::string& path)
    {
        std::string s = "ns\tthread\tcall\tindex\tvalue\topt\n";
        uint32_t n = nCalls_.load();
        if (n > (uint32_t)kMaxCalls) n = kMaxCalls;
        for (uint32_t i = 0; i < n; ++i)
        {
            const Call& c = callLog_[i];
            if (!c.ready.load(std::memory_order_acquire)) continue;
            s += fmt("%lld\t%llx\t%s\t%d\t%lld\t%g\n", (long long)c.ns, (unsigned long long)c.thread, opcodeName(c.opcode),
                     c.index, (long long)c.value, (double)c.opt);
        }
        if (nCalls_.load() > (uint32_t)kMaxCalls) s += fmt("# %u calls, first %d kept\n", nCalls_.load(), kMaxCalls);
        writeAtomically(path, s);
    }

    static const char* opcodeName(int op)
    {
        switch (op)
        {
            case -1: return "setParameter";
            case effOpen: return "open";
            case effClose: return "close";
            case effSetProgram: return "setProgram";
            case effGetProgram: return "getProgram";
            case effGetProgramName: return "getProgramName";
            case effGetParamLabel: return "getParamLabel";
            case effGetParamDisplay: return "getParamDisplay";
            case effGetParamName: return "getParamName";
            case effSetSampleRate: return "setSampleRate";
            case effSetBlockSize: return "setBlockSize";
            case effMainsChanged: return "mainsChanged";
            case effGetChunk: return "getChunk";
            case effSetChunk: return "setChunk";
            case effProcessEvents: return "processEvents";
            case effCanBeAutomated: return "canBeAutomated";
            case effGetProgramNameIndexed: return "getProgramNameIndexed";
            case effGetInputProperties: return "getInputProperties";
            case effGetOutputProperties: return "getOutputProperties";
            case effGetPlugCategory: return "getPlugCategory";
            case effSetSpeakerArrangement: return "setSpeakerArrangement";
            case effSetBypass: return "setBypass";
            case effGetEffectName: return "getEffectName";
            case effGetVendorString: return "getVendorString";
            case effGetProductString: return "getProductString";
            case effGetVendorVersion: return "getVendorVersion";
            case effCanDo: return "canDo";
            case effGetTailSize: return "getTailSize";
            case effGetVstVersion: return "getVstVersion";
            case effStartProcess: return "startProcess";
            case effStopProcess: return "stopProcess";
            case effSetProcessPrecision: return "setProcessPrecision";
            default: return "other";
        }
    }

    void writeStatus()
    {
        std::string text = fmt("fxprobe tap\nversion %d\nslot %d\nstate %s\n", kVersion, slot(), state_.c_str());
        if (capturing_) text += fmt("capture %s\nseq %ld\n", captureName_.c_str(), captureSeq_);
        else if (!doneCapture_.empty()) text += fmt("capture %s\nseq %ld\n", doneCapture_.c_str(), captureSeq_);
        text += fmt("rate %d\noutput %s\n", rate_.load(), getInt(kOutput) == 1 ? "mute" : "pass");
        if (!error_.empty()) text += "error " + error_ + "\n";
        if (text == lastStatus_) return;
        lastStatus_ = text;
        ensureDir(folder_ + "/status");
        writeAtomically(fmt("%s/status/tap-s%d.txt", folder_.c_str(), slot()), text);
    }

    // Shared between the threads.
    Ring<float> ring_;
    Ring<Tick> ticks_;
    std::atomic<long> recSeq_{0};
    std::atomic<int> recAction_{kNone};
    std::atomic<int64_t> recFrames_{0};
    std::atomic<long> doneSeq_{0};
    std::atomic<int64_t> overrun_{0};
    std::atomic<int64_t> tickDrops_{0};
    std::atomic<bool> timingOn_{false};
    std::atomic<int> policy_{-1};
    std::atomic<int> priority_{-1};
    std::atomic<int> rate_{44100};
    std::atomic<int64_t> calls_{0};
    std::atomic<uint32_t> hist_[kHistBuckets] = {};
    std::atomic<int> minBlock_{1 << 30};
    std::atomic<int> maxBlock_{0};
    Call callLog_[kMaxCalls];
    std::atomic<uint32_t> nCalls_{0};
    // Written by the audio thread before ack(), read by the worker after.
    double bpm_ = 0.0;
    bool playing_ = false;
    int64_t startNs_ = 0;
    int64_t endNs_ = 0;

    // Audio thread only.
    long seenSeq_ = 0;
    int64_t remaining_ = 0;
    bool recording_ = false;
    bool schedRead_ = false;

    // Worker only.
    WorkFolder work_{workFolder()};
    std::string folder_;
    std::unique_ptr<ControlWatcher> watcher_;
    WavWriter wav_;
    std::FILE* timingFile_ = nullptr;
    bool timingHeader_ = false;
    bool timingThis_ = false;
    bool capturing_ = false;
    long posted_ = 0;
    long postCount_ = 0;
    std::string base_;
    std::string captureName_;
    std::string doneCapture_;
    long captureSeq_ = -1;
    int64_t requested_ = 0;
    int64_t callsAtStart_ = 0;
    float peak_ = 0.0f;
    int64_t nonFinite_ = 0;
    std::string state_ = "idle";
    std::string error_;
    std::string lastStatus_;
    int autoSlot_ = 0;

    Worker worker_;   // last: stopped first, before the members it uses go
};

// The framework's entry point, with the dispatcher and setParameter wrapped so
// every host call is logged (for the timing report) before it is handled.
namespace {

VstIntPtr loggingDispatcher(AEffect* e, VstInt32 op, VstInt32 index, VstIntPtr value, void* ptr, float opt)
{
    if (auto* p = static_cast<FXProbeTap*>(static_cast<mpc::Plugin*>(e->object)))
        if (op != effClose && op != effEditIdle) p->logCall(op, index, (int64_t)value, opt);
    return mpc::detail::dispatcher(e, op, index, value, ptr, opt);
}

void loggingSetParameter(AEffect* e, VstInt32 index, float value)
{
    if (auto* p = static_cast<FXProbeTap*>(static_cast<mpc::Plugin*>(e->object))) p->logCall(-1, index, 0, value);
    mpc::detail::setParameter(e, index, value);
}

} // namespace

extern "C" MPC_EXPORT AEffect* VSTPluginMain(audioMasterCallback host)
{
    if (host != nullptr && host(nullptr, audioMasterVersion, 0, 0, nullptr, 0.0f) == 0) return nullptr;
    AEffect* e = mpc::detail::createEffect(new FXProbeTap(), host);
    if (e != nullptr)
    {
        e->dispatcher = &loggingDispatcher;
        e->setParameter = &loggingSetParameter;
    }
    return e;
}
