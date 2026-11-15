// FXProbeSource.cpp - an instrument that plays an fxlab test programme, so an
// effect on the same track (an AIR insert on the factory MPC) can be measured
// without anyone pressing anything.
//
// It plays a WAV from the work folder's programmes/ when the controller writes
// a "run" command to control.txt (see ../common/fxprobe.h), or on any MIDI
// note, or when the transport starts, as the Trigger parameter says. The
// programme is played sample for sample: no gain unless the command asks for
// one, no interpolation, no smoothing. Each programme begins with fxlab's sync
// chirp, so a recording can be lined up on it afterwards.
//
// Threads: a worker reads control.txt, loads programmes and writes the status
// file. The audio thread only reads buffers the worker has finished and
// handed over (two buffers, swapped with atomics).
//
// SPDX-License-Identifier: MIT

#include "mpc_plugin.h"
#include "dsp/wav.h"

#include "../../common/fxprobe.h"
#include "../../common/slots.h"

#include <memory>

using namespace fxprobe;

class FXProbeSource : public mpc::Plugin
{
public:
    enum Params { kSlot, kTrigger, kNumParams };
    enum Trigger { kCommandOnly, kCommandOrNote, kCommandOrPlay };
    enum State { kIdle, kArmed, kPlaying, kDone };
    enum Action { kNone, kRun, kStop };

    static mpc::PluginInfo makeInfo()
    {
        mpc::PluginInfo i;
        i.name = "FXProbe Source";
        i.vendor = "MPFree";
        i.uniqueId = MPC_FOURCC('M', 'F', 'p', 's');
        i.version = 1000;
        i.isInstrument = true;
        i.numInputs = 0;
        i.numOutputs = 2;
        i.tailSamples = 1;
        return i;
    }

    FXProbeSource() : mpc::Plugin(makeInfo())
    {
        addParam(mpc::choiceParam("slot", "Slot", slotChoices(), 0));
        addParam(mpc::choiceParam("trigger", "Trigger", {"Command", "Cmd+Note", "Cmd+Play"}, kCommandOrNote));
        autoSlot_ = claimSlot();
        folder_ = workFolder();
        worker_.start([this] { tick(); }, 20);
    }

    ~FXProbeSource() override
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

    void reset() override
    {
        state_.store(kIdle);
        pos_ = 0;
    }

    void process(mpc::ProcessContext& ctx) override
    {
        float* outL = ctx.outputs[0];
        float* outR = ctx.numOutputs > 1 ? ctx.outputs[1] : nullptr;
        const int n = ctx.numFrames;
        lastBlock_.store(n, std::memory_order_relaxed);

        // A command first, then the programme: the worker hands over the
        // programme before it posts the command, so reading in this order
        // always sees the programme the command was meant for.
        const long seq = cmdSeq_.load(std::memory_order_acquire);
        const int pub = published_.load(std::memory_order_acquire);
        const int st = state_.load(std::memory_order_relaxed);
        if (pub != cur_ && (st == kIdle || st == kDone || seq != seenSeq_))
        {
            cur_ = pub;
            inUse_.store(cur_, std::memory_order_release);
        }
        if (seq != seenSeq_)
        {
            seenSeq_ = seq;
            if (cmdAction_.load(std::memory_order_relaxed) == kRun)
                arm(cmdPreroll_.load(std::memory_order_relaxed), cmdGain_.load(std::memory_order_relaxed), seq);
            else
                finish(kIdle, seq);
        }

        const int trig = getInt(kTrigger);
        if (trig == kCommandOrNote)
            for (int i = 0; i < ctx.numMidiEvents; ++i)
                if (ctx.midi[i].isNoteOn()) { arm(0, 1.0f, -1); break; }
        if (trig == kCommandOrPlay)
        {
            bool playing = ctx.transport.valid && ctx.transport.playing;
            if (playing && !wasPlaying_) arm(0, 1.0f, -1);
            wasPlaying_ = playing;
        }

        const Prog* prog = cur_ >= 0 ? &progs_[cur_] : nullptr;
        for (int i = 0; i < n; ++i)
        {
            float l = 0.0f, r = 0.0f;
            int s = state_.load(std::memory_order_relaxed);
            if (s == kArmed)
            {
                if (preroll_ > 0) --preroll_;
                if (preroll_ == 0) { state_.store(kPlaying, std::memory_order_relaxed); s = kPlaying; pos_ = 0; }
            }
            if (s == kPlaying)
            {
                if (prog != nullptr && pos_ < (int64_t)prog->l.size())
                {
                    l = gain_ * prog->l[(size_t)pos_];
                    r = gain_ * prog->r[(size_t)pos_];
                    ++pos_;
                }
                else
                {
                    finish(kDone, runSeq_);
                }
            }
            outL[i] = l;
            if (outR) outR[i] = r;
        }
        posShared_.store(pos_, std::memory_order_relaxed);
    }

private:
    struct Prog
    {
        std::vector<float> l, r;
        std::string name;
    };

    // ---- audio thread -----------------------------------------------------
    void arm(int preroll, float gain, long seq)
    {
        preroll_ = preroll > 0 ? preroll : 0;
        gain_ = gain;
        pos_ = 0;
        runSeq_ = seq;
        stateSeq_.store(seq, std::memory_order_relaxed);
        state_.store(preroll_ > 0 ? kArmed : kPlaying, std::memory_order_release);
    }

    void finish(int state, long seq)
    {
        stateSeq_.store(seq, std::memory_order_relaxed);
        state_.store(state, std::memory_order_release);
    }

    // ---- worker -------------------------------------------------------------
    void tick()
    {
        if (!isDir(folder_)) return;
        if (!watcher_) watcher_.reset(new ControlWatcher(folder_));
        Command c;
        if (watcher_->poll(c) && c.forSlot(slot())) handle(c);
        writeStatus();
    }

    void handle(const Command& c)
    {
        error_.clear();
        if (c.action == "stop")
        {
            post(kStop, 0, 1.0f, c.seq);
            return;
        }
        if (c.action != "load" && c.action != "run")
        {
            error_ = "unknown action " + c.action;
            return;
        }
        if (!c.programme.empty() && c.programme != loadedName_)
        {
            if (!load(c.programme)) return;
        }
        if (c.action == "run")
        {
            if (loadedName_.empty()) { error_ = "no programme loaded"; return; }
            const int rate = rate_.load();
            post(kRun, (int)std::lround(c.preroll * rate), (float)std::pow(10.0, c.gainDb / 20.0), c.seq);
        }
    }

    void post(int action, int preroll, float gain, long seq)
    {
        cmdAction_.store(action, std::memory_order_relaxed);
        cmdPreroll_.store(preroll, std::memory_order_relaxed);
        cmdGain_.store(gain, std::memory_order_relaxed);
        cmdSeq_.store(seq, std::memory_order_release);
    }

    // Loads programmes/<name> into the buffer the audio thread is not using.
    bool load(const std::string& name)
    {
        if (!safeName(name)) { error_ = "bad programme name"; return false; }
        // Wait (up to a second) until the audio thread has taken the last
        // buffer we handed over and is not playing, so the other one is free.
        for (int i = 0; i < 100; ++i)
        {
            int s = state_.load();
            if (inUse_.load() == published_.load() && s != kArmed && s != kPlaying) break;
            if (i == 99) { error_ = "busy: audio thread did not take the last programme or is still playing"; return false; }
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
        }
        dsp::WavData w;
        if (!dsp::loadWav(folder_ + "/programmes/" + name, w)) { error_ = "cannot read programmes/" + name; return false; }
        const int rate = rate_.load();
        if ((int)std::lround(w.sampleRate) != rate)
        {
            error_ = fmt("programme is %.0f Hz, the host runs at %d Hz", w.sampleRate, rate);
            return false;
        }
        const int target = inUse_.load() == 0 ? 1 : 0;
        Prog& p = progs_[target];
        p.l = w.channels[0];
        p.r = w.numChannels() > 1 ? w.channels[1] : w.channels[0];
        p.name = name;
        published_.store(target, std::memory_order_release);
        loadedName_ = name;
        loadedFrames_ = (int64_t)p.l.size();
        return true;
    }

    void writeStatus()
    {
        static const char* names[] = {"idle", "armed", "playing", "done"};
        const int st = state_.load(std::memory_order_acquire);
        std::string text = fmt("fxprobe source\nversion %d\nslot %d\nstate %s\nseq %ld\nprogramme %s\nframes %lld\n",
                               kVersion, slot(), names[st & 3], stateSeq_.load(), loadedName_.c_str(),
                               (long long)loadedFrames_);
        text += fmt("rate %d\nblock %d\n", rate_.load(), lastBlock_.load());
        if (!error_.empty()) text += "error " + error_ + "\n";
        // Position changes all the time; it is only worth a write with something else.
        if (text == lastStatus_) return;
        lastStatus_ = text;
        ensureDir(folder_ + "/status");
        writeAtomically(fmt("%s/status/source-s%d.txt", folder_.c_str(), slot()),
                        text + fmt("pos %lld\n", (long long)posShared_.load()));
    }

    // Shared with the worker.
    Prog progs_[2];
    std::atomic<int> published_{-1};
    std::atomic<int> inUse_{-1};
    std::atomic<long> cmdSeq_{-1};
    std::atomic<int> cmdAction_{kNone};
    std::atomic<int> cmdPreroll_{0};
    std::atomic<float> cmdGain_{1.0f};
    std::atomic<int> state_{kIdle};
    std::atomic<long> stateSeq_{-1};
    std::atomic<int64_t> posShared_{0};
    std::atomic<int> rate_{44100};
    std::atomic<int> lastBlock_{0};

    // Audio thread only.
    int cur_ = -1;
    long seenSeq_ = -1;
    long runSeq_ = -1;
    int preroll_ = 0;
    float gain_ = 1.0f;
    int64_t pos_ = 0;
    bool wasPlaying_ = false;

    // Worker only.
    std::string folder_;
    std::unique_ptr<ControlWatcher> watcher_;
    std::string loadedName_;
    int64_t loadedFrames_ = 0;
    std::string error_;
    std::string lastStatus_;
    int autoSlot_ = 0;

    Worker worker_;   // last: stopped first, before the members it uses go
};

MPC_PLUGIN_ENTRY(FXProbeSource)
