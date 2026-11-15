// fxprobe.h - the pieces FXProbe Source and FXProbe Tap share: where the work
// folder is, the control file, status files, a lock-free ring, a float WAV
// writer and a small worker thread.
//
// Both plugins are driven from outside the MPC app through plain files in one
// work folder (by default "FXProbe" beside the drive's "VST2 Plugins" folder,
// or the folder a workfolder.txt pointer names; see workFolder()):
//
//   FXProbe/control.txt          written by the controller (fxprobe.py), read by every instance
//   FXProbe/programmes/*.wav     test programmes the Source plays (fxlab writes them)
//   FXProbe/captures/            what the Tap records: <capture>-s<slot>.wav plus a .txt report
//   FXProbe/status/<kind>-s<slot>.txt   each instance's state, rewritten when it changes
//
// The plugins never create the work folder; with no folder they do nothing at
// all. A control file present when an instance starts, or when the work folder
// appears (a stick put in), is ignored, so an old command never fires on
// project load or when a stick goes back in. Nothing here runs on the audio thread
// except the Ring push and the atomics.
//
// SPDX-License-Identifier: MIT

#ifndef FXPROBE_H
#define FXPROBE_H

#include "mpc_plugin.h"

#include <algorithm>
#include <atomic>
#include <cctype>
#include <cerrno>
#include <cstdarg>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <functional>
#include <string>
#include <thread>
#include <vector>

#include <dirent.h>
#include <sys/stat.h>
#include <unistd.h>

namespace fxprobe {

constexpr int kVersion = 1;
constexpr int kMaxSlots = 16;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

inline std::string parentOf(const std::string& p)
{
    size_t i = p.find_last_of('/');
    if (i == std::string::npos || i == 0) return std::string();
    return p.substr(0, i);
}

inline bool isDir(const std::string& p)
{
    struct stat st;
    return !p.empty() && ::stat(p.c_str(), &st) == 0 && S_ISDIR(st.st_mode);
}

inline bool isFile(const std::string& p)
{
    struct stat st;
    return !p.empty() && ::stat(p.c_str(), &st) == 0 && S_ISREG(st.st_mode);
}

// The marker fxprobe.sh setup leaves in a work folder. A pointer with a "*"
// in it matches only folders that hold one.
constexpr const char* kMarker = ".fxprobe-work";

// The work folder a pointer file names: its first line, an absolute path of
// printable characters with no ".." component, in which one whole component
// may be "*" (such as /media/*/FXProbe: any drive, whatever name Akai's
// system mounted it under). Anything else names no folder, so a damaged
// pointer leaves the plugin idle.
inline std::string pointedFolder(const std::string& text)
{
    std::string line = text.substr(0, text.find('\n'));
    while (!line.empty() && (line.back() == '\r' || line.back() == ' ' || line.back() == '\t')) line.pop_back();
    while (line.size() > 1 && line.back() == '/') line.pop_back();
    if (line.size() < 2 || line.size() > 400 || line[0] != '/') return std::string();
    for (char c : line)
        if ((unsigned char)c < 0x20 || c == 0x7f) return std::string();
    int stars = 0;
    size_t start = 1;
    while (start <= line.size())
    {
        size_t end = line.find('/', start);
        if (end == std::string::npos) end = line.size();
        std::string part = line.substr(start, end - start);
        if (part.empty() || part == "." || part == "..") return std::string();
        if (part.find('*') != std::string::npos && (part != "*" || ++stars > 1 || end == line.size()))
            return std::string();
        start = end + 1;
    }
    return line;
}

// Where the work folder is, in this order:
//   - FXPROBE_DIR, if set (desktop tests);
//   - the pointer in workfolder.txt in the plugin's own folder, which
//     fxprobe.sh install writes when it copies the plugins to the MPC's
//     internal drive, so that the work folder can be on a stick (whose
//     partitions Akai's system mounts noexec, so plugins don't load from it);
//   - <drive>/FXProbe for a plugin installed at <drive>/VST2 Plugins/<folder>.
// The answer may hold a "*" (see pointedFolder); WorkFolder resolves it.
inline std::string workFolder()
{
    if (const char* e = std::getenv("FXPROBE_DIR"))
        if (*e) return mpc::detail::normaliseSlashes(e);
    const std::string& plugin = mpc::pluginFolder();
    if (plugin.empty()) return std::string();
    std::string pointer;
    if (mpc::readTextFile(plugin + "/workfolder.txt", pointer)) return pointedFolder(pointer);
    std::string drive = parentOf(parentOf(plugin));
    if (drive.empty()) return std::string();
    return drive + "/FXProbe";
}

// Folders and files the plugins make are readable by everyone (0755, 0644)
// whatever the host's umask, which on the MPC is Akai's app's and unknown, so
// a workstation reads the stick without root.
inline bool ensureDir(const std::string& p)
{
    if (isDir(p)) return true;
    if (::mkdir(p.c_str(), 0755) == 0)
    {
        ::chmod(p.c_str(), 0755);
        return true;
    }
    return errno == EEXIST;
}

inline std::FILE* createFile(const std::string& path)
{
    std::FILE* f = std::fopen(path.c_str(), "wb");
    if (f) ::fchmod(::fileno(f), 0644);
    return f;
}

// A file name the controller gave us: no folders, no "..", printable.
inline bool safeName(const std::string& s)
{
    if (s.empty() || s.size() > 120 || s[0] == '.') return false;
    for (char c : s)
        if (!(std::isalnum((unsigned char)c) || c == '-' || c == '_' || c == '.')) return false;
    return true;
}

// Writes text to path through a temporary file and a rename, so a reader
// never sees half a file.
inline bool writeAtomically(const std::string& path, const std::string& text)
{
    std::string tmp = path + ".tmp";
    std::FILE* f = createFile(tmp);
    if (!f) return false;
    bool ok = std::fwrite(text.data(), 1, text.size(), f) == text.size();
    ok = (std::fclose(f) == 0) && ok;
    if (!ok || std::rename(tmp.c_str(), path.c_str()) != 0)
    {
        std::remove(tmp.c_str());
        return false;
    }
    return true;
}

inline int64_t monotonicNs()
{
    timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

// The folder a work-folder answer names now: the path itself if it is a
// folder, or, for a path with "*", the one folder that matches it and holds
// the marker. None, or more than one, is no folder.
inline std::string resolveWorkFolder(const std::string& spec)
{
    size_t star = spec.find("/*/");
    if (star == std::string::npos) return isDir(spec) ? spec : std::string();
    const std::string prefix = star == 0 ? std::string("/") : spec.substr(0, star);
    const std::string suffix = spec.substr(star + 3);
    std::string found;
    int matches = 0;
    if (DIR* d = ::opendir(prefix.c_str()))
    {
        while (dirent* e = ::readdir(d))
        {
            if (e->d_name[0] == '.') continue;
            std::string candidate = (prefix == "/" ? std::string() : prefix) + "/" + e->d_name + "/" + suffix;
            if (isDir(candidate) && isFile(candidate + "/" + kMarker))
            {
                found = candidate;
                ++matches;
            }
        }
        ::closedir(d);
    }
    return matches == 1 ? found : std::string();
}

// An instance's work folder as it comes and goes (a stick taken out and put
// back, perhaps under another mount name). Worker thread only.
class WorkFolder
{
public:
    explicit WorkFolder(std::string spec) : spec_(std::move(spec)) {}

    // The folder, or "" while there is none. A missing folder is looked for
    // again at most once a second.
    const std::string& current()
    {
        if (!path_.empty() && isDir(path_)) return path_;
        path_.clear();
        const int64_t now = monotonicNs();
        if (looked_ && now - lastLook_ < 1000000000LL) return path_;
        looked_ = true;
        lastLook_ = now;
        path_ = resolveWorkFolder(spec_);
        return path_;
    }

private:
    std::string spec_;
    std::string path_;
    bool looked_ = false;
    int64_t lastLook_ = 0;
};

// ---------------------------------------------------------------------------
// The control file
// ---------------------------------------------------------------------------
//
//   seq 17                 a new number means a new command
//   action run             load | run | stop
//   programme short.wav    Source: file in programmes/ (load, run)
//   gain -6                Source: dB applied to the programme (run; default 0)
//   preroll 0.5            Source: seconds of silence after the command before playing (run)
//   capture set-0017       Tap: captures/<capture>-s<slot>.wav (run)
//   seconds 14             Tap: how long to record (run)
//   timing 1               Tap: also log every process call (run; default 0)
//   slots 1,2,5            only these slots act (default: all)
//
// Unknown keys are ignored, so the format can grow.

struct Command
{
    long seq = -1;
    std::string action;
    std::string programme;
    double gainDb = 0.0;
    double preroll = 0.5;
    std::string capture;
    double seconds = 0.0;
    bool timing = false;
    std::vector<int> slots;   // empty: all

    bool forSlot(int slot) const
    {
        if (slots.empty()) return true;
        for (int s : slots)
            if (s == slot) return true;
        return false;
    }
};

inline bool parseCommand(const std::string& text, Command& c)
{
    c = Command();
    size_t pos = 0;
    while (pos < text.size())
    {
        size_t end = text.find('\n', pos);
        if (end == std::string::npos) end = text.size();
        std::string line = text.substr(pos, end - pos);
        pos = end + 1;
        while (!line.empty() && (line.back() == '\r' || line.back() == ' ' || line.back() == '\t')) line.pop_back();
        if (line.empty() || line[0] == '#') continue;
        size_t sp = line.find_first_of(" \t");
        std::string key = line.substr(0, sp);
        std::string val = sp == std::string::npos ? std::string() : line.substr(line.find_first_not_of(" \t", sp));
        if (key == "seq") c.seq = std::strtol(val.c_str(), nullptr, 10);
        else if (key == "action") c.action = val;
        else if (key == "programme") c.programme = val;
        else if (key == "gain") c.gainDb = std::strtod(val.c_str(), nullptr);
        else if (key == "preroll") c.preroll = std::strtod(val.c_str(), nullptr);
        else if (key == "capture") c.capture = val;
        else if (key == "seconds") c.seconds = std::strtod(val.c_str(), nullptr);
        else if (key == "timing") c.timing = std::strtol(val.c_str(), nullptr, 10) != 0;
        else if (key == "slots")
        {
            const char* p = val.c_str();
            while (*p)
            {
                char* q = nullptr;
                long s = std::strtol(p, &q, 10);
                if (q == p) { ++p; continue; }
                c.slots.push_back((int)s);
                p = q;
            }
        }
    }
    return c.seq >= 0;
}

// Watches control.txt and hands each new command over once. The command in
// the file when the watcher starts is treated as already seen.
class ControlWatcher
{
public:
    explicit ControlWatcher(std::string folder) : folder_(std::move(folder))
    {
        Command c;
        if (read(c)) lastSeq_ = c.seq;
    }

    // True when there is a new command, which is returned in c.
    bool poll(Command& c)
    {
        if (!read(c)) return false;
        if (c.seq == lastSeq_) return false;
        lastSeq_ = c.seq;
        return true;
    }

private:
    bool read(Command& c)
    {
        std::string path = folder_ + "/control.txt";
        struct stat st;
        if (::stat(path.c_str(), &st) != 0) return false;
        int64_t stamp = (int64_t)st.st_mtime * 1000000000LL + (int64_t)st.st_mtim.tv_nsec;
        if (stamp == lastStamp_ && (int64_t)st.st_size == lastSize_) return false;
        std::string text;
        if (!mpc::readTextFile(path, text)) return false;
        lastStamp_ = stamp;
        lastSize_ = (int64_t)st.st_size;
        return parseCommand(text, c);
    }

    std::string folder_;
    long lastSeq_ = -1;
    int64_t lastStamp_ = -1;
    int64_t lastSize_ = -1;
};

// ---------------------------------------------------------------------------
// Slots: which instance is which. "Auto" numbers instances in the order they
// were created in this process (each plugin library counts its own).
// ---------------------------------------------------------------------------

inline std::vector<std::string> slotChoices()
{
    std::vector<std::string> v{"Auto"};
    for (int i = 1; i <= kMaxSlots; ++i) v.push_back(std::to_string(i));
    return v;
}

// ---------------------------------------------------------------------------
// A single-producer single-consumer ring of T. The audio thread pushes, the
// worker pops. Capacity is a power of two, allocated once.
// ---------------------------------------------------------------------------

template <typename T>
class Ring
{
public:
    void allocate(size_t capacityPow2)
    {
        buf_.assign(capacityPow2, T());
        mask_ = capacityPow2 - 1;
        head_.store(0);
        tail_.store(0);
    }
    size_t capacity() const { return buf_.size(); }
    size_t freeSpace() const
    {
        return buf_.size() - (head_.load(std::memory_order_relaxed) - tail_.load(std::memory_order_acquire));
    }
    size_t used() const { return head_.load(std::memory_order_acquire) - tail_.load(std::memory_order_relaxed); }

    // Audio thread: all of n or nothing.
    bool push(const T* src, size_t n)
    {
        if (buf_.empty() || n > freeSpace()) return false;
        size_t h = head_.load(std::memory_order_relaxed);
        for (size_t i = 0; i < n; ++i) buf_[(h + i) & mask_] = src[i];
        head_.store(h + n, std::memory_order_release);
        return true;
    }
    bool push(const T& v) { return push(&v, 1); }

    // Worker: up to max items into dst; returns how many.
    size_t pop(T* dst, size_t max)
    {
        size_t t = tail_.load(std::memory_order_relaxed);
        size_t n = head_.load(std::memory_order_acquire) - t;
        if (n > max) n = max;
        for (size_t i = 0; i < n; ++i) dst[i] = buf_[(t + i) & mask_];
        tail_.store(t + n, std::memory_order_release);
        return n;
    }

private:
    std::vector<T> buf_;
    size_t mask_ = 0;
    std::atomic<size_t> head_{0};
    std::atomic<size_t> tail_{0};
};

// ---------------------------------------------------------------------------
// Float WAV writer (stereo, 32-bit float), header patched on close.
// ---------------------------------------------------------------------------

class WavWriter
{
public:
    ~WavWriter() { close(); }

    bool open(const std::string& path, int rate)
    {
        close();
        f_ = createFile(path);
        if (!f_) return false;
        rate_ = rate;
        frames_ = 0;
        writeHeader();
        return true;
    }
    bool isOpen() const { return f_ != nullptr; }
    uint64_t frames() const { return frames_; }

    // Interleaved stereo frames.
    bool write(const float* lr, size_t frames)
    {
        if (!f_) return false;
        size_t n = std::fwrite(lr, sizeof(float) * 2, frames, f_);
        frames_ += n;
        return n == frames;
    }

    bool close()
    {
        if (!f_) return true;
        std::fflush(f_);
        std::fseek(f_, 0, SEEK_SET);
        writeHeader();
        bool ok = std::fclose(f_) == 0;
        f_ = nullptr;
        return ok;
    }

private:
    void writeHeader()
    {
        const uint32_t dataBytes = (uint32_t)std::min<uint64_t>(frames_ * 8, 0xFFFFFFF0ull);
        uint8_t h[58];
        auto u16 = [&h](size_t p, uint32_t v) { h[p] = (uint8_t)v; h[p + 1] = (uint8_t)(v >> 8); };
        auto u32 = [&h](size_t p, uint32_t v) { for (int i = 0; i < 4; ++i) h[p + (size_t)i] = (uint8_t)(v >> (8 * i)); };
        std::memcpy(h, "RIFF", 4);
        u32(4, 50 + dataBytes);
        std::memcpy(h + 8, "WAVE", 4);
        std::memcpy(h + 12, "fmt ", 4);
        u32(16, 18);
        u16(20, 3);                    // IEEE float
        u16(22, 2);                    // channels
        u32(24, (uint32_t)rate_);
        u32(28, (uint32_t)rate_ * 8);  // bytes per second
        u16(32, 8);                    // block align
        u16(34, 32);                   // bits
        u16(36, 0);                    // cbSize
        std::memcpy(h + 38, "fact", 4);
        u32(42, 4);
        u32(46, (uint32_t)std::min<uint64_t>(frames_, 0xFFFFFFFFull));
        std::memcpy(h + 50, "data", 4);
        u32(54, dataBytes);
        std::fwrite(h, 1, sizeof(h), f_);
    }

    std::FILE* f_ = nullptr;
    int rate_ = 44100;
    uint64_t frames_ = 0;
};

// ---------------------------------------------------------------------------
// The worker: one thread per instance that calls tick() every period until
// the instance is destroyed.
// ---------------------------------------------------------------------------

class Worker
{
public:
    ~Worker() { stop(); }

    void start(std::function<void()> tick, int periodMs)
    {
        if (thread_.joinable()) return;
        stop_.store(false);
        thread_ = std::thread([this, tick, periodMs] {
            while (!stop_.load(std::memory_order_acquire))
            {
                tick();
                std::this_thread::sleep_for(std::chrono::milliseconds(periodMs));
            }
        });
    }

    void stop()
    {
        stop_.store(true, std::memory_order_release);
        if (thread_.joinable()) thread_.join();
    }

private:
    std::thread thread_;
    std::atomic<bool> stop_{false};
};

inline std::string fmt(const char* f, ...) __attribute__((format(printf, 1, 2)));
inline std::string fmt(const char* f, ...)
{
    char buf[512];
    va_list a;
    va_start(a, f);
    std::vsnprintf(buf, sizeof(buf), f, a);
    va_end(a);
    return buf;
}

} // namespace fxprobe

#endif // FXPROBE_H
