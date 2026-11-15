// chain_sim.cpp - a tiny VST2 host that runs plugins in series the way an MPC
// track does (instrument, then inserts in place), paced against the wall
// clock so the FXProbe workers see time pass. It exists to test FXProbe
// Source -> some effect -> FXProbe Tap end to end on a desktop, without an MPC.
//
//   chain_sim [--seconds N] [--speed X] [--block N] [--bpm B] [--playing]
//             [--set P:I=V]... source.so [effect.so ...] tap.so
//
// --speed is how many times faster than real time to run (default 8).
// --set gives plugin P (0 = first) parameter I the normalised value V.
//
// SPDX-License-Identifier: MIT

#include "mpc_vst2_abi.h"

#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <thread>
#include <vector>

#include <dlfcn.h>

namespace {

VstTimeInfo g_time{};
double g_rate = 44100.0;
int g_block = 128;

VstIntPtr hostCallback(AEffect*, VstInt32 opcode, VstInt32, VstIntPtr, void* ptr, float)
{
    switch (opcode)
    {
        case audioMasterVersion: return 2400;
        case audioMasterGetTime: return (VstIntPtr)&g_time;
        case audioMasterGetSampleRate: return (VstIntPtr)g_rate;
        case audioMasterGetBlockSize: return g_block;
        case audioMasterGetVendorString:
        case audioMasterGetProductString:
            if (ptr) std::strcpy(static_cast<char*>(ptr), "MPC 3");
            return 1;
        default: return 0;
    }
}

struct Loaded
{
    void* lib = nullptr;
    AEffect* fx = nullptr;
    std::string path;
};

AEffect* load(const std::string& path, Loaded& out)
{
    out.path = path;
    out.lib = dlopen(path.c_str(), RTLD_NOW | RTLD_LOCAL);
    if (!out.lib) { std::fprintf(stderr, "chain_sim: %s\n", dlerror()); return nullptr; }
    using Main = AEffect* (*)(audioMasterCallback);
    auto entry = reinterpret_cast<Main>(dlsym(out.lib, "VSTPluginMain"));
    if (!entry) { std::fprintf(stderr, "chain_sim: no VSTPluginMain in %s\n", path.c_str()); return nullptr; }
    out.fx = entry(&hostCallback);
    if (!out.fx || out.fx->magic != kEffectMagic) { std::fprintf(stderr, "chain_sim: %s did not start\n", path.c_str()); return nullptr; }
    AEffect* e = out.fx;
    e->dispatcher(e, effSetSampleRate, 0, 0, nullptr, (float)g_rate);
    e->dispatcher(e, effSetBlockSize, 0, g_block, nullptr, 0.0f);
    e->dispatcher(e, effOpen, 0, 0, nullptr, 0.0f);
    e->dispatcher(e, effMainsChanged, 0, 1, nullptr, 0.0f);
    e->dispatcher(e, effStartProcess, 0, 0, nullptr, 0.0f);
    return e;
}

} // namespace

int main(int argc, char** argv)
{
    double seconds = 10.0, speed = 8.0, bpm = 120.0;
    bool playing = false;
    std::vector<std::string> paths;
    struct Set { int plugin, index; float value; };
    std::vector<Set> sets;
    for (int i = 1; i < argc; ++i)
    {
        std::string a = argv[i];
        auto next = [&]() -> const char* { if (i + 1 >= argc) { std::fprintf(stderr, "chain_sim: %s needs a value\n", a.c_str()); std::exit(2); } return argv[++i]; };
        if (a == "--seconds") seconds = std::atof(next());
        else if (a == "--speed") speed = std::atof(next());
        else if (a == "--block") g_block = std::atoi(next());
        else if (a == "--bpm") bpm = std::atof(next());
        else if (a == "--playing") playing = true;
        else if (a == "--set")
        {
            Set s{};
            if (std::sscanf(next(), "%d:%d=%f", &s.plugin, &s.index, &s.value) != 3) { std::fprintf(stderr, "chain_sim: --set P:I=V\n"); return 2; }
            sets.push_back(s);
        }
        else paths.push_back(a);
    }
    if (paths.size() < 2) { std::fprintf(stderr, "usage: chain_sim [options] source.so [effect.so ...] tap.so\n"); return 2; }

    std::vector<Loaded> chain(paths.size());
    for (size_t i = 0; i < paths.size(); ++i)
        if (!load(paths[i], chain[i])) return 1;
    for (const Set& s : sets)
        if (s.plugin >= 0 && s.plugin < (int)chain.size()) chain[(size_t)s.plugin].fx->setParameter(chain[(size_t)s.plugin].fx, s.index, s.value);

    std::vector<float> l((size_t)g_block), r((size_t)g_block), zl((size_t)g_block), zr((size_t)g_block);
    float* io[2] = {l.data(), r.data()};
    float* zero[2] = {zl.data(), zr.data()};
    VstEvents none{};

    g_time.sampleRate = g_rate;
    g_time.tempo = bpm;
    g_time.timeSigNumerator = 4;
    g_time.timeSigDenominator = 4;
    g_time.flags = kVstTempoValid | kVstPpqPosValid | kVstTimeSigValid | kVstBarsValid | (playing ? kVstTransportPlaying : 0);

    const long long total = (long long)(seconds * g_rate);
    const auto t0 = std::chrono::steady_clock::now();
    for (long long done = 0; done < total; done += g_block)
    {
        g_time.samplePos = (double)done;
        g_time.ppqPos = done / g_rate * bpm / 60.0;
        g_time.barStartPos = 4.0 * (long long)(g_time.ppqPos / 4.0);
        std::memset(zl.data(), 0, sizeof(float) * zl.size());
        std::memset(zr.data(), 0, sizeof(float) * zr.size());
        AEffect* src = chain[0].fx;
        src->dispatcher(src, effProcessEvents, 0, 0, &none, 0.0f);
        src->processReplacing(src, zero, io, g_block);
        for (size_t i = 1; i < chain.size(); ++i)
            chain[i].fx->processReplacing(chain[i].fx, io, io, g_block);   // in place, like the MPC
        auto due = t0 + std::chrono::duration<double>((done + g_block) / g_rate / speed);
        std::this_thread::sleep_until(due);
    }

    for (Loaded& c : chain)
    {
        c.fx->dispatcher(c.fx, effStopProcess, 0, 0, nullptr, 0.0f);
        c.fx->dispatcher(c.fx, effMainsChanged, 0, 0, nullptr, 0.0f);
        c.fx->dispatcher(c.fx, effClose, 0, 0, nullptr, 0.0f);
        dlclose(c.lib);
    }
    return 0;
}
