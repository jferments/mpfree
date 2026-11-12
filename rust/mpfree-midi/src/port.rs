//! External MIDI ports: the 5-pin DIN jacks and USB devices on the USB-A
//! socket. The MPC's own controller (pads, knobs, buttons) is not one of
//! them.
//!
//! A [`PortId`] is a small number the routers index by. [`Ports`] hands
//! them out by name, so a keyboard unplugged and plugged back in gets its
//! old number, and with it its track inputs, MIDI Learn mappings and
//! clock settings.
//!
//! Which ALSA device and substream a port is (for the DIN jacks, which
//! cable of the controller's USB-MIDI device) is the app's to set, once
//! the owner's read-only capture has shown it. Nothing here opens a port.

/// External ports at once: DIN in/out plus up to seven USB devices.
pub const MAX_PORTS: usize = 8;

/// Index of an external port, below [`MAX_PORTS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PortId(pub u8);

/// The DIN jacks always have this id.
pub const DIN: PortId = PortId(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortKind {
    /// The 5-pin MIDI In and Out jacks: 31250 bit/s, so output is paced
    /// ([`crate::output::DinPacer`]).
    Din,
    /// A class-compliant USB-MIDI device on the USB-A socket.
    UsbHost,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInfo {
    pub id: PortId,
    pub kind: PortKind,
    /// The name the device reports (a USB keyboard's ALSA card name), or
    /// "MIDI" for the DIN jacks.
    pub name: String,
    pub connected: bool,
    pub has_in: bool,
    pub has_out: bool,
}

/// The port table, kept by the app's control thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ports {
    ports: Vec<PortInfo>,
}

impl Default for Ports {
    fn default() -> Self {
        Self::new()
    }
}

impl Ports {
    /// The DIN jacks, not connected until the app has opened them.
    pub fn new() -> Ports {
        Ports {
            ports: vec![PortInfo {
                id: DIN,
                kind: PortKind::Din,
                name: "MIDI".to_string(),
                connected: false,
                has_in: true,
                has_out: true,
            }],
        }
    }

    /// A USB device appeared: its old id if one with this name was seen
    /// before, else the next free one. None when all [`MAX_PORTS`] ids are
    /// taken by connected devices. A disconnected one is reused then.
    pub fn plugged(&mut self, name: &str, has_in: bool, has_out: bool) -> Option<PortId> {
        if let Some(p) = self
            .ports
            .iter_mut()
            .find(|p| p.kind == PortKind::UsbHost && p.name == name && !p.connected)
        {
            p.connected = true;
            p.has_in = has_in;
            p.has_out = has_out;
            return Some(p.id);
        }
        let info = |id| PortInfo {
            id,
            kind: PortKind::UsbHost,
            name: name.to_string(),
            connected: true,
            has_in,
            has_out,
        };
        if self.ports.len() < MAX_PORTS {
            let id = PortId(self.ports.len() as u8);
            self.ports.push(info(id));
            return Some(id);
        }
        let slot = self
            .ports
            .iter_mut()
            .find(|p| p.kind == PortKind::UsbHost && !p.connected)?;
        *slot = info(slot.id);
        Some(slot.id)
    }

    /// A USB device went away (its id stays reserved for it).
    pub fn unplugged(&mut self, id: PortId) {
        if let Some(p) = self
            .ports
            .iter_mut()
            .find(|p| p.id == id && p.kind == PortKind::UsbHost)
        {
            p.connected = false;
        }
    }

    /// The DIN jacks opened (true) or closed.
    pub fn set_din(&mut self, connected: bool) {
        self.ports[0].connected = connected;
    }

    pub fn get(&self, id: PortId) -> Option<&PortInfo> {
        self.ports.iter().find(|p| p.id == id)
    }

    pub fn all(&self) -> &[PortInfo] {
        &self.ports
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keyboard_gets_its_id_back() {
        let mut p = Ports::new();
        let a = p.plugged("Keystep 37", true, true).unwrap();
        let b = p.plugged("nanoKONTROL2", true, true).unwrap();
        assert_eq!((a, b), (PortId(1), PortId(2)));
        p.unplugged(a);
        assert_eq!(p.plugged("Keystep 37", true, false), Some(a));
        assert!(!p.get(a).unwrap().has_out);
        // Two of the same model at once get two ids.
        assert_eq!(p.plugged("Keystep 37", true, true), Some(PortId(3)));
    }

    #[test]
    fn full_table_reuses_a_gone_device() {
        let mut p = Ports::new();
        for i in 1..MAX_PORTS {
            assert_eq!(
                p.plugged(&format!("dev{i}"), true, true),
                Some(PortId(i as u8))
            );
        }
        assert_eq!(p.plugged("one more", true, true), None);
        p.unplugged(PortId(4));
        assert_eq!(p.plugged("one more", true, true), Some(PortId(4)));
        assert_eq!(p.get(PortId(4)).unwrap().name, "one more");
        // The DIN jacks are never handed out.
        p.unplugged(DIN);
        assert_eq!(p.get(DIN).unwrap().kind, PortKind::Din);
    }
}
