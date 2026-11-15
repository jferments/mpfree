// slots.h - "Auto" slot numbers: each instance claims the lowest number no
// live instance of the same plugin holds, and gives it back when destroyed.
// A host that creates a throwaway instance while scanning plugins therefore
// does not shift the numbers of the real ones.
//
// SPDX-License-Identifier: MIT

#ifndef FXPROBE_SLOTS_H
#define FXPROBE_SLOTS_H

#include <atomic>
#include <cstdint>

namespace fxprobe {

// One per plugin library (each library has its own copy of this static).
inline std::atomic<uint32_t>& claimedSlots()
{
    static std::atomic<uint32_t> bits{0};
    return bits;
}

// Returns 1..16, or 0 when all sixteen are taken.
inline int claimSlot()
{
    std::atomic<uint32_t>& bits = claimedSlots();
    uint32_t cur = bits.load();
    for (;;)
    {
        int free = 0;
        for (int i = 0; i < 16; ++i)
            if ((cur & (1u << i)) == 0) { free = i + 1; break; }
        if (free == 0) return 0;
        if (bits.compare_exchange_weak(cur, cur | (1u << (free - 1)))) return free;
    }
}

inline void releaseSlot(int slot)
{
    if (slot >= 1 && slot <= 16) claimedSlots().fetch_and(~(1u << (slot - 1)));
}

} // namespace fxprobe

#endif
