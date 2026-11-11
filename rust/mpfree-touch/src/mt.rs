//! M11: host-only Linux Type-B multitouch decoder.
//!
//! Pure state machine over already-read `input_event` triples. It performs no
//! system calls and opens no devices: the caller feeds events (synthetic in
//! tests, a recorded stream later, a real evdev reader only after a separately
//! gated milestone) and gets coherent per-frame results back.
//!
//! Protocol summary (Documentation/input/multi-touch-protocol.rst):
//! - `ABS_MT_SLOT n` selects the slot that following `ABS_MT_*` values apply to.
//!   The kernel only sends it when the slot changes, so the current slot is
//!   state that persists across frames.
//! - `ABS_MT_TRACKING_ID id >= 0` starts a contact in the current slot;
//!   `-1` ends it. A slot may switch straight from one id to another.
//! - Per-slot values are filtered for duplicates by the kernel, so a new
//!   contact that lands on the same X as the previous contact in that slot does
//!   not resend X. Positions therefore persist per slot, across contacts.
//! - `SYN_REPORT` commits a frame. `SYN_DROPPED` means the kernel buffer
//!   overflowed: everything up to and including the next `SYN_REPORT` must be
//!   discarded and the full slot state re-read (`EVIOCGMTSLOTS`).
//!
//! The decoder allocates only in [`MtDecoder::new`]; `push` reuses buffers.

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_ABS: u16 = 0x03;
pub const SYN_REPORT: u16 = 0;
pub const SYN_DROPPED: u16 = 3;
pub const BTN_TOUCH: u16 = 0x14a;

const ABS_MT_SLOT: u16 = crate::ABS_MT_SLOT as u16;
const ABS_MT_POSITION_X: u16 = crate::ABS_MT_POSITION_X as u16;
const ABS_MT_POSITION_Y: u16 = crate::ABS_MT_POSITION_Y as u16;
const ABS_MT_TRACKING_ID: u16 = crate::ABS_MT_TRACKING_ID as u16;

/// Linux's own hard ceiling on multitouch slots per device.
pub const MAX_SLOTS: usize = 64;

/// One `struct input_event`, with the timestamp flattened to microseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputEvent {
    pub time_us: u64,
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}

impl InputEvent {
    pub const fn new(time_us: u64, kind: u16, code: u16, value: i32) -> Self {
        Self {
            time_us,
            kind,
            code,
            value,
        }
    }
}

/// Decoder-assigned contact identity. Unlike the kernel tracking id (which
/// wraps at 65535 on the ILI2117 and can be reused), this never repeats within
/// one decoder's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContactId(pub u64);

/// A contact as published in a committed frame. Coordinates are raw device
/// units; orientation and scaling belong to [`crate::pointer::AxisMap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Contact {
    pub id: ContactId,
    pub slot: u8,
    pub tracking_id: i32,
    pub raw_x: i32,
    pub raw_y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Down,
    Move,
    Up,
    /// The contact's end was not observed (sync loss). Consumers must abort
    /// whatever the contact was doing rather than treat it as a release.
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactEvent {
    pub phase: Phase,
    pub contact: Contact,
    pub time_us: u64,
}

/// What a single `push` produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Event absorbed into the pending frame (or ignored).
    Pending,
    /// A frame was committed: read [`MtDecoder::events`] and [`MtDecoder::contacts`].
    Frame,
    /// Sync was lost and the post-drop `SYN_REPORT` has arrived. The caller
    /// should now call [`MtDecoder::resync`] with a fresh `EVIOCGMTSLOTS`
    /// snapshot, or [`MtDecoder::resync_without_snapshot`]. If it does
    /// neither, the next `push` performs the latter implicitly.
    ResyncRequired,
}

/// Counters for diagnostics; never used for control flow by callers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecoderStats {
    pub frames: u64,
    pub sync_drops: u64,
    pub ignored_events: u64,
    pub out_of_range_slots: u64,
    pub btn_touch_mismatches: u64,
}

/// One slot as read back by the caller after a sync drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotSnapshot {
    pub tracking_id: i32,
    pub raw_x: i32,
    pub raw_y: i32,
}

/// Full multitouch state, in the shape `EVIOCGABS(ABS_MT_SLOT).value` plus
/// `EVIOCGMTSLOTS` for TRACKING_ID / POSITION_X / POSITION_Y return it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MtSnapshot {
    pub current_slot: i32,
    pub slots: Vec<SlotSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    InvalidSlotCount,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Slot {
    /// Committed contact, if published.
    live: Option<Contact>,
    /// Kernel tracking id as of the pending frame (None = no contact, or
    /// unknown after sync loss without a snapshot).
    tracking_id: Option<i32>,
    /// An end (-1) or a direct id-to-id switch happened this frame.
    ended: bool,
    x: Option<i32>,
    y: Option<i32>,
    /// After an unrecovered drop the slot's contact may be a finger that went
    /// down before the drop. Its updates are ignored until a fresh tracking id.
    stale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncState {
    InSync,
    /// Discarding until the next SYN_REPORT.
    Dropping,
    AwaitingResync,
}

#[derive(Debug, Clone)]
pub struct MtDecoder {
    slots: Vec<Slot>,
    /// None after a drop without snapshot: kernel's current slot is unknown.
    current_slot: Option<usize>,
    sync: SyncState,
    next_id: u64,
    btn_touch: Option<bool>,
    events: Vec<ContactEvent>,
    /// Cancels queued by an implicit resync, to be kept by the next commit.
    carry_events: bool,
    contacts: Vec<Contact>,
    stats: DecoderStats,
}

impl MtDecoder {
    /// `slot_count` comes from `ABS_MT_SLOT` metadata (10 on the measured
    /// MPC One, see M10); it is not assumed here.
    pub fn new(slot_count: usize) -> Result<Self, ConfigError> {
        if slot_count == 0 || slot_count > MAX_SLOTS {
            return Err(ConfigError::InvalidSlotCount);
        }
        Ok(Self {
            slots: vec![Slot::default(); slot_count],
            // Linux starts with slot 0 selected.
            current_slot: Some(0),
            sync: SyncState::InSync,
            next_id: 1,
            btn_touch: None,
            events: Vec::with_capacity(slot_count * 2),
            carry_events: false,
            contacts: Vec::with_capacity(slot_count),
            stats: DecoderStats::default(),
        })
    }

    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Transitions of the last committed frame, ordered by slot; within a
    /// slot an end (Up/Cancel) always precedes a new Down.
    pub fn events(&self) -> &[ContactEvent] {
        &self.events
    }

    /// Every contact live after the last committed frame, ordered by slot.
    pub fn contacts(&self) -> &[Contact] {
        &self.contacts
    }

    pub fn stats(&self) -> DecoderStats {
        self.stats
    }

    pub fn push(&mut self, ev: InputEvent) -> Step {
        match self.sync {
            SyncState::Dropping => {
                if ev.kind == EV_SYN && ev.code == SYN_REPORT {
                    self.sync = SyncState::AwaitingResync;
                    return Step::ResyncRequired;
                }
                if !(ev.kind == EV_SYN && ev.code == SYN_DROPPED) {
                    self.stats.ignored_events += 1;
                }
                return Step::Pending;
            }
            SyncState::AwaitingResync => {
                // Caller did not resync explicitly; fall back to cancelling.
                // The cancel frame is reported through `events()` only if the
                // event below is itself a SYN_REPORT; otherwise the cancels
                // are merged into the next committed frame.
                self.cancel_all_into_pending(ev.time_us);
                self.sync = SyncState::InSync;
            }
            SyncState::InSync => {}
        }

        match (ev.kind, ev.code) {
            (EV_SYN, SYN_REPORT) => {
                self.commit(ev.time_us);
                Step::Frame
            }
            (EV_SYN, SYN_DROPPED) => {
                self.stats.sync_drops += 1;
                self.sync = SyncState::Dropping;
                self.discard_pending();
                Step::Pending
            }
            (EV_ABS, ABS_MT_SLOT) => {
                match usize::try_from(ev.value)
                    .ok()
                    .filter(|&s| s < self.slots.len())
                {
                    Some(s) => self.current_slot = Some(s),
                    None => {
                        self.stats.out_of_range_slots += 1;
                        self.current_slot = None;
                    }
                }
                Step::Pending
            }
            (EV_ABS, ABS_MT_TRACKING_ID) => {
                let Some(slot) = self.slot_mut() else {
                    self.stats.ignored_events += 1;
                    return Step::Pending;
                };
                if ev.value < 0 {
                    slot.ended |= slot.live.is_some();
                    slot.tracking_id = None;
                    slot.stale = false;
                } else if slot.tracking_id != Some(ev.value) || slot.stale {
                    if slot.live.is_some() {
                        slot.ended = true;
                    }
                    slot.tracking_id = Some(ev.value);
                    slot.stale = false;
                }
                Step::Pending
            }
            (EV_ABS, ABS_MT_POSITION_X | ABS_MT_POSITION_Y) => {
                let is_x = ev.code == ABS_MT_POSITION_X;
                match self.slot_mut() {
                    Some(slot) => {
                        // A stale slot's values are still the kernel's true
                        // per-slot state, so keep them: the kernel will not
                        // resend a value a later contact happens to share.
                        // They are never published while the slot is stale.
                        let stale = slot.stale;
                        if is_x {
                            slot.x = Some(ev.value);
                        } else {
                            slot.y = Some(ev.value);
                        }
                        if stale {
                            self.stats.ignored_events += 1;
                        }
                    }
                    None => self.stats.ignored_events += 1,
                }
                Step::Pending
            }
            (EV_KEY, BTN_TOUCH) => {
                self.btn_touch = Some(ev.value != 0);
                Step::Pending
            }
            // Single-touch emulation (ABS_X/ABS_Y), pressure, EV_MSC
            // timestamps etc. are redundant with the MT stream.
            _ => {
                self.stats.ignored_events += 1;
                Step::Pending
            }
        }
    }

    /// Restore exact state from a snapshot read after `ResyncRequired`.
    /// Contacts whose tracking id survived keep their [`ContactId`] (and get a
    /// Move if they moved); vanished ones get Up; replaced ones get Up + Down.
    /// The result is published as a frame.
    pub fn resync(&mut self, snapshot: &MtSnapshot, time_us: u64) -> Step {
        self.discard_pending();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let snap = snapshot.slots.get(index).copied().unwrap_or(SlotSnapshot {
                tracking_id: -1,
                raw_x: 0,
                raw_y: 0,
            });
            let new_id = (snap.tracking_id >= 0).then_some(snap.tracking_id);
            let old_id = slot.live.map(|c| c.tracking_id);
            slot.ended = new_id != old_id && slot.live.is_some();
            slot.tracking_id = new_id;
            slot.x = Some(snap.raw_x);
            slot.y = Some(snap.raw_y);
            slot.stale = false;
        }
        self.current_slot = usize::try_from(snapshot.current_slot)
            .ok()
            .filter(|&s| s < self.slots.len());
        self.sync = SyncState::InSync;
        self.commit(time_us);
        Step::Frame
    }

    /// Seed a decoder from the device's state as read at open
    /// (`EVIOCGABS(ABS_MT_SLOT).value` plus `EVIOCGMTSLOTS`).
    ///
    /// A fresh decoder otherwise assumes slot 0 is selected and every slot is
    /// empty, but the kernel sends `ABS_MT_SLOT` only when the slot changes
    /// and filters repeated per-slot values, so a reader that opens while the
    /// kernel's current slot is not 0 would file the next contact under the
    /// wrong slot. Seeding fixes the current slot and each slot's last
    /// position.
    ///
    /// Fingers already on the screen at open are not published: their slots
    /// are ignored until they lift and a fresh contact starts there, so a
    /// resting finger cannot press whatever lies under it when the reader
    /// starts. Any contacts this decoder had already published are cancelled;
    /// the result is `Frame` if there were some (read [`Self::events`]),
    /// otherwise `Pending`.
    pub fn seed(&mut self, snapshot: &MtSnapshot, time_us: u64) -> Step {
        let had_contacts = !self.contacts.is_empty();
        self.cancel_all_into_pending(time_us);
        // The cancels (if any) are this call's own result, not carried.
        self.carry_events = false;
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let snap = snapshot.slots.get(index).copied();
            *slot = Slot {
                live: None,
                tracking_id: snap.and_then(|s| (s.tracking_id >= 0).then_some(s.tracking_id)),
                ended: false,
                x: snap.map(|s| s.raw_x),
                y: snap.map(|s| s.raw_y),
                stale: snap.is_some_and(|s| s.tracking_id >= 0),
            };
        }
        self.current_slot = usize::try_from(snapshot.current_slot)
            .ok()
            .filter(|&s| s < self.slots.len());
        self.sync = SyncState::InSync;
        if had_contacts {
            Step::Frame
        } else {
            Step::Pending
        }
    }

    /// Recover when no snapshot can be read: Cancel every live contact and
    /// ignore each slot until it reports a fresh tracking id. Published as a
    /// frame.
    pub fn resync_without_snapshot(&mut self, time_us: u64) -> Step {
        self.discard_pending();
        self.cancel_all_into_pending(time_us);
        self.sync = SyncState::InSync;
        self.commit(time_us);
        Step::Frame
    }

    fn slot_mut(&mut self) -> Option<&mut Slot> {
        let index = self.current_slot?;
        self.slots.get_mut(index)
    }

    fn cancel_all_into_pending(&mut self, time_us: u64) {
        self.events.clear();
        for slot in &mut self.slots {
            if let Some(contact) = slot.live.take() {
                self.events.push(ContactEvent {
                    phase: Phase::Cancel,
                    contact,
                    time_us,
                });
            }
            *slot = Slot {
                stale: true,
                ..Slot::default()
            };
        }
        self.current_slot = None;
        self.btn_touch = None;
        self.contacts.clear();
        // Cancels are carried into the next commit, which must not clear them.
        self.carry_events = true;
    }

    /// Roll each slot's pending edits back to its committed contact.
    fn discard_pending(&mut self) {
        for slot in &mut self.slots {
            if !slot.stale {
                slot.tracking_id = slot.live.map(|c| c.tracking_id);
                if let Some(c) = slot.live {
                    slot.x = Some(c.raw_x);
                    slot.y = Some(c.raw_y);
                }
            }
            slot.ended = false;
        }
    }

    fn commit(&mut self, time_us: u64) {
        if !std::mem::take(&mut self.carry_events) {
            self.events.clear();
        }
        self.contacts.clear();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.ended {
                if let Some(contact) = slot.live.take() {
                    self.events.push(ContactEvent {
                        phase: Phase::Up,
                        contact,
                        time_us,
                    });
                }
            }
            match (slot.tracking_id, slot.x, slot.y, slot.stale) {
                (Some(tracking_id), Some(raw_x), Some(raw_y), false) => match slot.live.as_mut() {
                    Some(contact) => {
                        if contact.raw_x != raw_x || contact.raw_y != raw_y {
                            contact.raw_x = raw_x;
                            contact.raw_y = raw_y;
                            self.events.push(ContactEvent {
                                phase: Phase::Move,
                                contact: *contact,
                                time_us,
                            });
                        }
                    }
                    None => {
                        let contact = Contact {
                            id: ContactId(self.next_id),
                            // Slot count is capped at 64, so this cannot truncate.
                            slot: index as u8,
                            tracking_id,
                            raw_x,
                            raw_y,
                        };
                        self.next_id += 1;
                        slot.live = Some(contact);
                        self.events.push(ContactEvent {
                            phase: Phase::Down,
                            contact,
                            time_us,
                        });
                    }
                },
                // Contact started but a coordinate is still unknown (only
                // possible after sync loss): hold it back until both arrive.
                (Some(_), _, _, false) => {}
                // No contact, or a stale slot.
                _ => {
                    if let Some(contact) = slot.live.take() {
                        self.events.push(ContactEvent {
                            phase: Phase::Up,
                            contact,
                            time_us,
                        });
                    }
                }
            }
            slot.ended = false;
            if let Some(c) = slot.live {
                self.contacts.push(c);
            }
        }
        if let Some(down) = self.btn_touch {
            if down == self.contacts.is_empty() {
                self.stats.btn_touch_mismatches += 1;
            }
        }
        self.stats.frames += 1;
    }
}
