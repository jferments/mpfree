// MPFree's CLAP wrapper for Airwindows effects.
//
// One .clap file holds the effects of one Airwindows Consolidated collection
// ("Recommended" by default), each as its own CLAP effect: stereo in and out,
// its own parameters (0 to 1, as Airwindows has them), its own value text and
// a small state. No editor: MPFree draws parameters itself.
//
// The effects come from Airwindows Consolidated (airwin2rack, MIT), fetched by
// tools/plugin-pack/build.py at a pinned commit. This file is MPFree's, under
// the repository's license, and builds only against that checkout.

#include <clap/clap.h>

#include "AirwinRegistry.h"

#include <algorithm>
#include <atomic>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <string>
#include <vector>

#ifndef MPFREE_AW_COLLECTION
#define MPFREE_AW_COLLECTION "Recommended"
#endif
#ifndef MPFREE_AW_VERSION
#define MPFREE_AW_VERSION "0"
#endif

namespace
{

// Airwindows registers only effects with at most 10 parameters.
constexpr int MAX_PARAMS = 10;
constexpr uint32_t STATE_MAGIC = 0x57414d4d; // "MMAW" read as little-endian bytes
constexpr uint32_t STATE_VERSION = 1;

struct Desc
{
    int reg{0};
    std::string id, name, description;
    int params{0};
    std::vector<std::string> paramNames;
    std::vector<float> defaults;
    std::vector<const char *> features;
    clap_plugin_descriptor d{};
};

std::vector<std::unique_ptr<Desc>> descs;
int initCount = 0;

const char *categoryFeature(const std::string &cat)
{
    if (cat == "Reverb")
        return CLAP_PLUGIN_FEATURE_REVERB;
    if (cat == "Ambience")
        return CLAP_PLUGIN_FEATURE_DELAY;
    if (cat == "Distortion" || cat == "Saturation" || cat == "Clipping" || cat == "Lo-Fi")
        return CLAP_PLUGIN_FEATURE_DISTORTION;
    if (cat == "Filter" || cat == "XYZ Filters" || cat == "Biquads")
        return CLAP_PLUGIN_FEATURE_FILTER;
    if (cat == "Utility" || cat == "Dithers")
        return CLAP_PLUGIN_FEATURE_UTILITY;
    if (cat == "Consoles")
        return CLAP_PLUGIN_FEATURE_MIXING;
    return nullptr;
}

std::string lower(std::string s)
{
    for (auto &c : s)
        if (c >= 'A' && c <= 'Z')
            c = (char)(c - 'A' + 'a');
    return s;
}

std::string trimmed(const char *t)
{
    std::string s(t);
    auto b = s.find_first_not_of(" \t");
    if (b == std::string::npos)
        return {};
    auto e = s.find_last_not_of(" \t");
    return s.substr(b, e - b + 1);
}

std::unique_ptr<AirwinConsolidatedBase> makeFx(const Desc &d, float rate)
{
    auto fx = AirwinRegistry::registry[d.reg].generator();
    if (fx)
        fx->setSampleRate(rate);
    return fx;
}

bool buildDescs()
{
    // Airwindows code reads the sample rate in some constructors.
    AirwinConsolidatedBase::defaultSampleRate = 48000.f;
    auto &reg = AirwinRegistry::registry;
    std::vector<int> picked;
    for (int i = 0; i < (int)reg.size(); ++i)
    {
        const auto &c = reg[i].collections;
        if (std::find(c.begin(), c.end(), MPFREE_AW_COLLECTION) != c.end())
            picked.push_back(i);
    }
    // Grouped by category, then by name, so a long list reads in groups.
    std::sort(picked.begin(), picked.end(), [&](int a, int b) {
        if (reg[a].category != reg[b].category)
            return reg[a].category < reg[b].category;
        return reg[a].name < reg[b].name;
    });
    for (int i : picked)
    {
        auto d = std::make_unique<Desc>();
        d->reg = i;
        d->name = reg[i].name;
        d->id = "mpfree.airwindows." + lower(reg[i].name);
        d->description = reg[i].whatText;
        auto fx = makeFx(*d, 48000.f);
        if (!fx)
            continue;
        d->params = std::min(reg[i].nParams, MAX_PARAMS);
        for (int p = 0; p < d->params; ++p)
        {
            char t[kVstMaxParamStrLen * 4] = {};
            fx->getParameterName(p, t);
            d->paramNames.push_back(trimmed(t));
            d->defaults.push_back(std::clamp(fx->getParameter(p), 0.f, 1.f));
        }
        d->features.push_back(CLAP_PLUGIN_FEATURE_AUDIO_EFFECT);
        if (auto f = categoryFeature(reg[i].category))
            d->features.push_back(f);
        d->features.push_back(CLAP_PLUGIN_FEATURE_STEREO);
        d->features.push_back(nullptr);
        d->d.clap_version = CLAP_VERSION;
        d->d.id = d->id.c_str();
        d->d.name = d->name.c_str();
        d->d.vendor = "Airwindows";
        d->d.url = "https://www.airwindows.com/";
        d->d.manual_url = "https://www.airwindows.com/";
        d->d.support_url = "";
        d->d.version = MPFREE_AW_VERSION;
        d->d.description = d->description.c_str();
        d->d.features = d->features.data();
        descs.push_back(std::move(d));
    }
    return !descs.empty();
}

struct Plugin
{
    clap_plugin plugin{};
    const clap_host *host{nullptr};
    const Desc *desc{nullptr};
    float rate{48000.f};
    bool active{false};

    // The audio thread's effect. A spare waits for reset(), which swaps it in
    // without allocating; the main thread makes the next spare and frees the
    // one swapped out.
    AirwinConsolidatedBase *fx{nullptr};
    std::atomic<AirwinConsolidatedBase *> spare{nullptr};
    std::atomic<AirwinConsolidatedBase *> retired{nullptr};
    // The main thread's copy, for value text only.
    std::unique_ptr<AirwinConsolidatedBase> display;

    std::atomic<float> values[MAX_PARAMS];
    // A state load bumps `wanted`; the audio thread applies every value when it
    // differs from `applied`.
    std::atomic<uint32_t> wanted{0};
    uint32_t applied{0};

    std::vector<float> inL, inR, junk;

    void applyAll(AirwinConsolidatedBase *to)
    {
        for (int p = 0; p < desc->params; ++p)
            to->setParameter(p, values[p].load(std::memory_order_relaxed));
    }

    void paramEvent(const clap_event_header *h)
    {
        if (h->space_id != CLAP_CORE_EVENT_SPACE_ID || h->type != CLAP_EVENT_PARAM_VALUE)
            return;
        auto ev = reinterpret_cast<const clap_event_param_value *>(h);
        if (ev->param_id >= (clap_id)desc->params)
            return;
        float v = std::clamp((float)ev->value, 0.f, 1.f);
        values[ev->param_id].store(v, std::memory_order_relaxed);
        if (fx)
            fx->setParameter((int)ev->param_id, v);
    }
};

Plugin *self(const clap_plugin *p) { return static_cast<Plugin *>(p->plugin_data); }

// ---------------------------------------------------------------- plugin

bool pInit(const clap_plugin *p)
{
    auto s = self(p);
    s->display = makeFx(*s->desc, s->rate);
    return s->display != nullptr;
}

void pDestroy(const clap_plugin *p)
{
    auto s = self(p);
    delete s->fx;
    delete s->spare.exchange(nullptr);
    delete s->retired.exchange(nullptr);
    delete s;
}

bool pActivate(const clap_plugin *p, double rate, uint32_t, uint32_t maxFrames)
{
    auto s = self(p);
    s->rate = (float)rate;
    delete s->fx;
    delete s->spare.exchange(nullptr);
    delete s->retired.exchange(nullptr);
    s->fx = makeFx(*s->desc, s->rate).release();
    if (!s->fx)
        return false;
    s->spare.store(makeFx(*s->desc, s->rate).release());
    s->applyAll(s->fx);
    s->applied = s->wanted.load();
    s->inL.assign(maxFrames, 0.f);
    s->inR.assign(maxFrames, 0.f);
    s->junk.assign(maxFrames, 0.f);
    s->active = true;
    return true;
}

void pDeactivate(const clap_plugin *p)
{
    auto s = self(p);
    s->active = false;
}

bool pStartProcessing(const clap_plugin *) { return true; }
void pStopProcessing(const clap_plugin *) {}

void pReset(const clap_plugin *p)
{
    auto s = self(p);
    if (s->retired.load() != nullptr)
        return; // the last swap isn't cleaned up yet
    auto fresh = s->spare.exchange(nullptr);
    if (!fresh)
        return;
    s->applyAll(fresh);
    s->retired.store(s->fx);
    s->fx = fresh;
    s->host->request_callback(s->host);
}

clap_process_status pProcess(const clap_plugin *p, const clap_process *proc)
{
    auto s = self(p);
    const uint32_t n = proc->frames_count;
    if (!s->fx || proc->audio_outputs_count < 1 || n > s->inL.size())
        return CLAP_PROCESS_ERROR;
    if (s->wanted.load(std::memory_order_acquire) != s->applied)
    {
        s->applied = s->wanted.load(std::memory_order_acquire);
        s->applyAll(s->fx);
    }

    // Copy the input first: the host may give the same buffers for both.
    const clap_audio_buffer *in = proc->audio_inputs_count > 0 ? &proc->audio_inputs[0] : nullptr;
    const clap_audio_buffer &out = proc->audio_outputs[0];
    if (in && in->channel_count > 0 && in->data32)
    {
        std::memcpy(s->inL.data(), in->data32[0], n * sizeof(float));
        const float *r = in->channel_count > 1 ? in->data32[1] : in->data32[0];
        std::memcpy(s->inR.data(), r, n * sizeof(float));
    }
    else
    {
        std::fill(s->inL.begin(), s->inL.begin() + n, 0.f);
        std::fill(s->inR.begin(), s->inR.begin() + n, 0.f);
    }
    if (out.channel_count < 1 || !out.data32)
        return CLAP_PROCESS_ERROR;
    float *outL = out.data32[0];
    float *outR = out.channel_count > 1 ? out.data32[1] : s->junk.data();

    // Run up to each parameter change, then apply it, so it lands on its frame.
    const clap_input_events *evs = proc->in_events;
    const uint32_t count = evs ? evs->size(evs) : 0;
    uint32_t pos = 0, e = 0;
    while (pos < n)
    {
        uint32_t until = n;
        while (e < count)
        {
            auto h = evs->get(evs, e);
            if (h->time > pos)
            {
                until = std::min<uint32_t>(h->time, n);
                break;
            }
            s->paramEvent(h);
            ++e;
        }
        float *ins[2] = {s->inL.data() + pos, s->inR.data() + pos};
        float *outs[2] = {outL + pos, outR + pos};
        s->fx->processReplacing(ins, outs, (VstInt32)(until - pos));
        pos = until;
    }
    for (; e < count; ++e)
        s->paramEvent(evs->get(evs, e));
    if (out.channel_count > 2)
        for (uint32_t c = 2; c < out.channel_count; ++c)
            std::memset(out.data32[c], 0, n * sizeof(float));
    return CLAP_PROCESS_CONTINUE;
}

void pOnMainThread(const clap_plugin *p)
{
    auto s = self(p);
    delete s->retired.exchange(nullptr);
    if (s->active && s->spare.load() == nullptr)
    {
        auto fresh = makeFx(*s->desc, s->rate).release();
        AirwinConsolidatedBase *none = nullptr;
        if (!s->spare.compare_exchange_strong(none, fresh))
            delete fresh;
    }
}

// ---------------------------------------------------------------- params

uint32_t prCount(const clap_plugin *p) { return (uint32_t)self(p)->desc->params; }

bool prInfo(const clap_plugin *p, uint32_t index, clap_param_info *info)
{
    auto d = self(p)->desc;
    if (index >= (uint32_t)d->params)
        return false;
    std::memset(info, 0, sizeof(*info));
    info->id = index;
    info->flags = CLAP_PARAM_IS_AUTOMATABLE;
    std::snprintf(info->name, sizeof(info->name), "%s", d->paramNames[index].c_str());
    info->min_value = 0.0;
    info->max_value = 1.0;
    info->default_value = d->defaults[index];
    return true;
}

bool prValue(const clap_plugin *p, clap_id id, double *value)
{
    auto s = self(p);
    if (id >= (clap_id)s->desc->params)
        return false;
    *value = s->values[id].load(std::memory_order_relaxed);
    return true;
}

bool prToText(const clap_plugin *p, clap_id id, double value, char *out, uint32_t size)
{
    auto s = self(p);
    if (id >= (clap_id)s->desc->params || !s->display || size == 0)
        return false;
    char disp[kVstMaxParamStrLen * 4] = {}, label[kVstMaxParamStrLen * 4] = {};
    s->display->setParameter((int)id, std::clamp((float)value, 0.f, 1.f));
    s->display->getParameterDisplay((int)id, disp);
    s->display->getParameterLabel((int)id, label);
    auto d = trimmed(disp), l = trimmed(label);
    std::snprintf(out, size, "%s%s%s", d.c_str(), l.empty() ? "" : " ", l.c_str());
    return true;
}

bool prFromText(const clap_plugin *p, clap_id id, const char *text, double *value)
{
    auto s = self(p);
    if (id >= (clap_id)s->desc->params || !s->display)
        return false;
    float v = 0.f;
    if (s->display->canConvertParameterTextToValue((int)id) &&
        s->display->parameterTextToValue((int)id, text, v))
    {
        *value = std::clamp(v, 0.f, 1.f);
        return true;
    }
    char *end = nullptr;
    double d = std::strtod(text, &end);
    if (end == text)
        return false;
    *value = std::clamp(d, 0.0, 1.0);
    return true;
}

void prFlush(const clap_plugin *p, const clap_input_events *in, const clap_output_events *)
{
    auto s = self(p);
    const uint32_t count = in ? in->size(in) : 0;
    for (uint32_t i = 0; i < count; ++i)
        s->paramEvent(in->get(in, i));
}

const clap_plugin_params paramsExt = {prCount, prInfo, prValue, prToText, prFromText, prFlush};

// ---------------------------------------------------------------- state

bool writeAll(const clap_ostream *st, const void *data, size_t size)
{
    auto b = static_cast<const uint8_t *>(data);
    while (size > 0)
    {
        int64_t w = st->write(st, b, size);
        if (w <= 0)
            return false;
        b += w;
        size -= (size_t)w;
    }
    return true;
}

bool readAll(const clap_istream *st, void *data, size_t size)
{
    auto b = static_cast<uint8_t *>(data);
    while (size > 0)
    {
        int64_t r = st->read(st, b, size);
        if (r <= 0)
            return false;
        b += r;
        size -= (size_t)r;
    }
    return true;
}

// Little-endian: magic, version, name length, name, value count, values.
bool stSave(const clap_plugin *p, const clap_ostream *st)
{
    auto s = self(p);
    const auto &name = s->desc->name;
    uint32_t head[3] = {STATE_MAGIC, STATE_VERSION, (uint32_t)name.size()};
    if (!writeAll(st, head, sizeof(head)) || !writeAll(st, name.data(), name.size()))
        return false;
    uint32_t n = (uint32_t)s->desc->params;
    if (!writeAll(st, &n, sizeof(n)))
        return false;
    for (uint32_t i = 0; i < n; ++i)
    {
        float v = s->values[i].load(std::memory_order_relaxed);
        if (!writeAll(st, &v, sizeof(v)))
            return false;
    }
    return true;
}

bool stLoad(const clap_plugin *p, const clap_istream *st)
{
    auto s = self(p);
    uint32_t head[3];
    if (!readAll(st, head, sizeof(head)) || head[0] != STATE_MAGIC || head[1] != STATE_VERSION ||
        head[2] != s->desc->name.size())
        return false;
    std::string name(head[2], '\0');
    if (!readAll(st, name.data(), name.size()) || name != s->desc->name)
        return false;
    uint32_t n = 0;
    if (!readAll(st, &n, sizeof(n)) || n != (uint32_t)s->desc->params)
        return false;
    float got[MAX_PARAMS];
    for (uint32_t i = 0; i < n; ++i)
        if (!readAll(st, &got[i], sizeof(float)) || !(got[i] >= 0.f && got[i] <= 1.f))
            return false;
    for (uint32_t i = 0; i < n; ++i)
        s->values[i].store(got[i], std::memory_order_relaxed);
    s->wanted.fetch_add(1, std::memory_order_release);
    if (!s->active && s->fx)
    {
        s->applyAll(s->fx);
        s->applied = s->wanted.load();
    }
    return true;
}

const clap_plugin_state stateExt = {stSave, stLoad};

// ---------------------------------------------------------------- audio ports

uint32_t apCount(const clap_plugin *, bool) { return 1; }

bool apGet(const clap_plugin *, uint32_t index, bool isInput, clap_audio_port_info *info)
{
    if (index != 0)
        return false;
    std::memset(info, 0, sizeof(*info));
    info->id = 0;
    std::snprintf(info->name, sizeof(info->name), "%s", isInput ? "In" : "Out");
    info->flags = CLAP_AUDIO_PORT_IS_MAIN;
    info->channel_count = 2;
    info->port_type = CLAP_PORT_STEREO;
    info->in_place_pair = 0;
    return true;
}

const clap_plugin_audio_ports audioPortsExt = {apCount, apGet};

const void *pExtension(const clap_plugin *, const char *id)
{
    if (!std::strcmp(id, CLAP_EXT_PARAMS))
        return &paramsExt;
    if (!std::strcmp(id, CLAP_EXT_STATE))
        return &stateExt;
    if (!std::strcmp(id, CLAP_EXT_AUDIO_PORTS))
        return &audioPortsExt;
    return nullptr;
}

// ---------------------------------------------------------------- factory

uint32_t fCount(const clap_plugin_factory *) { return (uint32_t)descs.size(); }

const clap_plugin_descriptor *fDescriptor(const clap_plugin_factory *, uint32_t index)
{
    return index < descs.size() ? &descs[index]->d : nullptr;
}

const clap_plugin *fCreate(const clap_plugin_factory *, const clap_host *host, const char *id)
{
    if (!host || !clap_version_is_compatible(host->clap_version))
        return nullptr;
    for (const auto &d : descs)
    {
        if (d->id != id)
            continue;
        auto s = new Plugin();
        s->host = host;
        s->desc = d.get();
        for (int i = 0; i < d->params; ++i)
            s->values[i].store(d->defaults[i]);
        s->plugin.desc = &d->d;
        s->plugin.plugin_data = s;
        s->plugin.init = pInit;
        s->plugin.destroy = pDestroy;
        s->plugin.activate = pActivate;
        s->plugin.deactivate = pDeactivate;
        s->plugin.start_processing = pStartProcessing;
        s->plugin.stop_processing = pStopProcessing;
        s->plugin.reset = pReset;
        s->plugin.process = pProcess;
        s->plugin.get_extension = pExtension;
        s->plugin.on_main_thread = pOnMainThread;
        return &s->plugin;
    }
    return nullptr;
}

const clap_plugin_factory factory = {fCount, fDescriptor, fCreate};

bool entryInit(const char *)
{
    if (initCount++ == 0 && descs.empty())
        return buildDescs();
    return !descs.empty();
}

void entryDeinit()
{
    if (initCount > 0)
        --initCount;
}

const void *entryFactory(const char *id)
{
    return std::strcmp(id, CLAP_PLUGIN_FACTORY_ID) ? nullptr : &factory;
}

} // namespace

extern "C" CLAP_EXPORT const clap_plugin_entry_t clap_entry = {CLAP_VERSION_INIT, entryInit,
                                                              entryDeinit, entryFactory};
