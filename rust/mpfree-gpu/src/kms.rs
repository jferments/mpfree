//! The panel through DRM/KMS: the few kernel calls that show buffers the
//! GPU drew, with no library in between.
//!
//! [`KmsDisplay::open`] finds the one screen that is lit now (its connector,
//! the CRTC that drives it and the mode it runs), makes buffers of exactly
//! that size in the panel's own format (XRGB8888) and hands them out as
//! dma-bufs for the GPU to draw into. Nothing on the screen changes until
//! [`Display::show`](crate::scanout::Display::show): the first one points
//! the CRTC at a buffer of ours, with the same mode it already runs, and
//! after that each one is a page flip at the next vertical blank, so a
//! frame is never seen half drawn.
//!
//! Every change is in RAM. The CRTC's own settings are read first and put
//! back when the display is dropped (or [`KmsDisplay::restore`] is called),
//! which shows the framebuffer console's picture again (`/dev/fb0`). If
//! the program dies before that, the kernel removes our buffers when it
//! closes our file, and the framebuffer emulation sets its own picture
//! again when the last DRM file closes; a power cycle undoes everything.
//!
//! The structures are the kernel's (`include/uapi/drm/drm.h`,
//! `drm_mode.h`); they have the same layout on 32-bit ARM and on x86-64,
//! and the tests check their sizes and the request numbers against the
//! kernel headers' values.

use std::ffi::c_void;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::scanout::Display;

/// DRM_FORMAT_XRGB8888 ('X' 'R' '2' '4'): 32 bits a pixel, blue in the
/// lowest byte, the top byte unused. The panel's framebuffer format.
pub const XRGB8888: u32 = u32::from_le_bytes(*b"XR24");

/// How long a page flip may take before the display counts as stuck. The
/// panel refreshes about 60 times a second, so a flip takes at most ~17 ms.
pub const FLIP_TIMEOUT: Duration = Duration::from_millis(250);

// ------------------------------------------------------------ the kernel's structures

/// struct drm_mode_modeinfo: one display mode (timings and a name).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModeInfo {
    pub clock: u32,
    pub hdisplay: u16,
    pub hsync_start: u16,
    pub hsync_end: u16,
    pub htotal: u16,
    pub hskew: u16,
    pub vdisplay: u16,
    pub vsync_start: u16,
    pub vsync_end: u16,
    pub vtotal: u16,
    pub vscan: u16,
    pub vrefresh: u32,
    pub flags: u32,
    pub kind: u32,
    pub name: [u8; 32],
}

impl ModeInfo {
    /// The mode's name, as the kernel gives it (such as `800x1280`).
    pub fn name(&self) -> String {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        String::from_utf8_lossy(&self.name[..end]).into_owned()
    }
}

/// struct drm_mode_card_res.
#[repr(C)]
#[derive(Default)]
struct CardRes {
    fb_id_ptr: u64,
    crtc_id_ptr: u64,
    connector_id_ptr: u64,
    encoder_id_ptr: u64,
    count_fbs: u32,
    count_crtcs: u32,
    count_connectors: u32,
    count_encoders: u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

/// struct drm_mode_get_connector.
#[repr(C)]
#[derive(Default)]
struct GetConnector {
    encoders_ptr: u64,
    modes_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,
    count_modes: u32,
    count_props: u32,
    count_encoders: u32,
    encoder_id: u32,
    connector_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    connection: u32,
    mm_width: u32,
    mm_height: u32,
    subpixel: u32,
    pad: u32,
}

/// struct drm_mode_get_encoder.
#[repr(C)]
#[derive(Default)]
struct GetEncoder {
    encoder_id: u32,
    encoder_type: u32,
    crtc_id: u32,
    possible_crtcs: u32,
    possible_clones: u32,
}

/// struct drm_mode_crtc.
#[repr(C)]
#[derive(Default)]
struct ModeCrtc {
    set_connectors_ptr: u64,
    count_connectors: u32,
    crtc_id: u32,
    fb_id: u32,
    x: u32,
    y: u32,
    gamma_size: u32,
    mode_valid: u32,
    mode: ModeInfo,
}

/// struct drm_mode_create_dumb.
#[repr(C)]
#[derive(Default)]
struct CreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

/// struct drm_mode_map_dumb.
#[repr(C)]
#[derive(Default)]
struct MapDumb {
    handle: u32,
    pad: u32,
    offset: u64,
}

/// struct drm_mode_destroy_dumb.
#[repr(C)]
#[derive(Default)]
struct DestroyDumb {
    handle: u32,
}

/// struct drm_mode_fb_cmd2.
#[repr(C)]
#[derive(Default)]
struct FbCmd2 {
    fb_id: u32,
    width: u32,
    height: u32,
    pixel_format: u32,
    flags: u32,
    handles: [u32; 4],
    pitches: [u32; 4],
    offsets: [u32; 4],
    modifier: [u64; 4],
}

/// struct drm_prime_handle.
#[repr(C)]
#[derive(Default)]
struct PrimeHandle {
    handle: u32,
    flags: u32,
    fd: i32,
}

/// struct drm_mode_crtc_page_flip.
#[repr(C)]
#[derive(Default)]
struct PageFlip {
    crtc_id: u32,
    fb_id: u32,
    flags: u32,
    reserved: u32,
    user_data: u64,
}

/// struct drm_event, the head of every event read from the card.
const EVENT_HEAD: usize = 8;
/// DRM_EVENT_FLIP_COMPLETE.
const EVENT_FLIP_COMPLETE: u32 = 0x02;
/// DRM_MODE_PAGE_FLIP_EVENT.
const PAGE_FLIP_EVENT: u32 = 0x01;
/// DRM_CLOEXEC | DRM_RDWR for a dma-buf.
const PRIME_FLAGS: u32 = libc::O_CLOEXEC as u32 | libc::O_RDWR as u32;
/// DRM_MODE_CONNECTED.
const CONNECTED: u32 = 1;

/// _IOWR('d', nr, size), as the kernel's <asm-generic/ioctl.h> builds it.
const fn iowr(nr: u32, size: usize) -> u32 {
    (3 << 30) | ((size as u32) << 16) | ((b'd' as u32) << 8) | nr
}

/// _IO('d', nr).
const fn io(nr: u32) -> u32 {
    ((b'd' as u32) << 8) | nr
}

use std::mem::size_of;
pub(crate) const SET_MASTER: u32 = io(0x1e);
pub(crate) const DROP_MASTER: u32 = io(0x1f);
pub(crate) const PRIME_HANDLE_TO_FD: u32 = iowr(0x2d, size_of::<PrimeHandle>());
pub(crate) const GET_RESOURCES: u32 = iowr(0xa0, size_of::<CardRes>());
pub(crate) const GET_CRTC: u32 = iowr(0xa1, size_of::<ModeCrtc>());
pub(crate) const SET_CRTC: u32 = iowr(0xa2, size_of::<ModeCrtc>());
pub(crate) const GET_ENCODER: u32 = iowr(0xa6, size_of::<GetEncoder>());
pub(crate) const GET_CONNECTOR: u32 = iowr(0xa7, size_of::<GetConnector>());
pub(crate) const RM_FB: u32 = iowr(0xaf, size_of::<u32>());
pub(crate) const PAGE_FLIP: u32 = iowr(0xb0, size_of::<PageFlip>());
pub(crate) const CREATE_DUMB: u32 = iowr(0xb2, size_of::<CreateDumb>());
pub(crate) const MAP_DUMB: u32 = iowr(0xb3, size_of::<MapDumb>());
pub(crate) const DESTROY_DUMB: u32 = iowr(0xb4, size_of::<DestroyDumb>());
pub(crate) const ADD_FB2: u32 = iowr(0xb8, size_of::<FbCmd2>());

// ------------------------------------------------------------ the card

/// An open DRM card (a display controller's primary node, such as
/// `/dev/dri/card0`).
pub struct Card {
    fd: OwnedFd,
}

/// The CRTC's settings as read: what it shows, where, and in which mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crtc {
    pub id: u32,
    pub fb: u32,
    pub x: u32,
    pub y: u32,
    /// None when the CRTC is off.
    pub mode: Option<ModeInfo>,
}

/// A buffer in the display controller's memory, made for showing.
#[derive(Clone, Copy, Debug)]
pub struct Dumb {
    pub handle: u32,
    pub pitch: u32,
    pub size: u64,
    pub width: u32,
    pub height: u32,
}

/// A buffer mapped into this process (read and write), unmapped on drop.
pub struct Mapping {
    ptr: *mut c_void,
    len: usize,
}

impl Mapping {
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `ptr` maps `len` bytes for as long as `self` lives.
        unsafe { std::slice::from_raw_parts(self.ptr as *const u8, self.len) }
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: `ptr` maps `len` writable bytes for as long as `self`
        // lives, and `&mut self` keeps this the only view.
        unsafe { std::slice::from_raw_parts_mut(self.ptr as *mut u8, self.len) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: a mapping this value made.
        unsafe { libc::munmap(self.ptr, self.len) };
    }
}

fn errno() -> std::io::Error {
    std::io::Error::last_os_error()
}

impl Card {
    /// Opens the card for reading and writing. Opening changes nothing on
    /// the screen.
    pub fn open(path: &Path) -> Result<Card, String> {
        let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| format!("{}: a path with a NUL in it", path.display()))?;
        // SAFETY: a plain open(2) of a NUL-terminated path.
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(format!("{}: {}", path.display(), errno()));
        }
        // SAFETY: a file descriptor this function just opened.
        Ok(Card {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        use std::os::fd::AsFd;
        self.fd.as_fd()
    }

    /// One DRM request, tried again while a signal interrupts it (as
    /// libdrm's drmIoctl does).
    fn ioctl<T>(&self, what: &str, request: u32, arg: &mut T) -> Result<(), String> {
        loop {
            // SAFETY: `arg` is the structure `request` names (its size is in
            // the request number), alive and writable for the call.
            let r = unsafe {
                libc::ioctl(
                    self.fd.as_raw_fd(),
                    request as _,
                    arg as *mut T as *mut c_void,
                )
            };
            if r == 0 {
                return Ok(());
            }
            let e = errno();
            match e.raw_os_error() {
                Some(libc::EINTR) | Some(libc::EAGAIN) => continue,
                _ => return Err(format!("{what}: {e}")),
            }
        }
    }

    /// A request that takes no argument (SET_MASTER, DROP_MASTER).
    fn ioctl_none(&self, what: &str, request: u32) -> Result<(), String> {
        // SAFETY: requests of this kind ignore the argument.
        let r = unsafe { libc::ioctl(self.fd.as_raw_fd(), request as _, 0usize) };
        if r == 0 {
            Ok(())
        } else {
            Err(format!("{what}: {}", errno()))
        }
    }

    /// The card's CRTC and connector ids.
    pub fn resources(&self) -> Result<(Vec<u32>, Vec<u32>), String> {
        // Ask for the counts, then the ids; again if they grew in between.
        for _ in 0..4 {
            let mut res = CardRes::default();
            self.ioctl("reading the card's resources", GET_RESOURCES, &mut res)?;
            let mut crtcs = vec![0u32; res.count_crtcs as usize];
            let mut connectors = vec![0u32; res.count_connectors as usize];
            let mut again = CardRes {
                crtc_id_ptr: crtcs.as_mut_ptr() as usize as u64,
                connector_id_ptr: connectors.as_mut_ptr() as usize as u64,
                count_crtcs: res.count_crtcs,
                count_connectors: res.count_connectors,
                ..CardRes::default()
            };
            self.ioctl("reading the card's resources", GET_RESOURCES, &mut again)?;
            if again.count_crtcs == res.count_crtcs
                && again.count_connectors == res.count_connectors
            {
                return Ok((crtcs, connectors));
            }
        }
        Err("the card's resources kept changing".into())
    }

    /// A connector's state as the kernel last found it (connected or not,
    /// and its current encoder), without asking the panel again: room for
    /// one mode is given, which is what keeps the kernel from probing (as
    /// libdrm's drmModeGetConnectorCurrent does).
    pub fn connector(&self, id: u32) -> Result<(bool, u32), String> {
        let mut scratch = ModeInfo::default();
        let mut c = GetConnector {
            connector_id: id,
            count_modes: 1,
            modes_ptr: &mut scratch as *mut ModeInfo as usize as u64,
            ..GetConnector::default()
        };
        self.ioctl(&format!("reading connector {id}"), GET_CONNECTOR, &mut c)?;
        Ok((c.connection == CONNECTED, c.encoder_id))
    }

    /// The CRTC an encoder drives now (0 for none).
    pub fn encoder_crtc(&self, id: u32) -> Result<u32, String> {
        let mut e = GetEncoder {
            encoder_id: id,
            ..GetEncoder::default()
        };
        self.ioctl(&format!("reading encoder {id}"), GET_ENCODER, &mut e)?;
        Ok(e.crtc_id)
    }

    /// A CRTC's settings.
    pub fn crtc(&self, id: u32) -> Result<Crtc, String> {
        let mut c = ModeCrtc {
            crtc_id: id,
            ..ModeCrtc::default()
        };
        self.ioctl(&format!("reading CRTC {id}"), GET_CRTC, &mut c)?;
        Ok(Crtc {
            id,
            fb: c.fb_id,
            x: c.x,
            y: c.y,
            mode: (c.mode_valid != 0).then_some(c.mode),
        })
    }

    /// Points `crtc` at framebuffer `fb`, scanning from (x, y), driving
    /// `connector` in `mode`. Needs the card's master.
    pub fn set_crtc(
        &self,
        crtc: u32,
        fb: u32,
        x: u32,
        y: u32,
        connector: u32,
        mode: &ModeInfo,
    ) -> Result<(), String> {
        let mut connectors = [connector];
        let mut c = ModeCrtc {
            set_connectors_ptr: connectors.as_mut_ptr() as usize as u64,
            count_connectors: 1,
            crtc_id: crtc,
            fb_id: fb,
            x,
            y,
            mode_valid: 1,
            mode: *mode,
            ..ModeCrtc::default()
        };
        self.ioctl(&format!("setting CRTC {crtc}"), SET_CRTC, &mut c)
    }

    /// Asks for `fb` on `crtc` at the next vertical blank, with an event
    /// when it happens. Needs the card's master.
    pub fn page_flip(&self, crtc: u32, fb: u32, user_data: u64) -> Result<(), String> {
        let mut f = PageFlip {
            crtc_id: crtc,
            fb_id: fb,
            flags: PAGE_FLIP_EVENT,
            reserved: 0,
            user_data,
        };
        self.ioctl("flipping the page", PAGE_FLIP, &mut f)
    }

    /// Waits up to `timeout` for a page flip to complete; its user data.
    pub fn wait_flip(&self, timeout: Duration) -> Result<u64, String> {
        let until = Instant::now() + timeout;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!(
                    "the panel did not take the frame (no page flip in {} ms)",
                    timeout.as_millis()
                ));
            }
            let mut p = libc::pollfd {
                fd: self.fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one pollfd, alive for the call.
            let n = unsafe { libc::poll(&mut p, 1, left.as_millis().max(1) as i32) };
            if n < 0 {
                let e = errno();
                if e.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(format!("waiting for the page flip: {e}"));
            }
            if n == 0 {
                continue;
            }
            let mut buf = [0u8; 1024];
            // SAFETY: reading into a buffer of buf.len() bytes.
            let got = unsafe {
                libc::read(
                    self.fd.as_raw_fd(),
                    buf.as_mut_ptr() as *mut c_void,
                    buf.len(),
                )
            };
            if got < 0 {
                let e = errno();
                if matches!(e.raw_os_error(), Some(libc::EINTR) | Some(libc::EAGAIN)) {
                    continue;
                }
                return Err(format!("reading the card's events: {e}"));
            }
            if let Some(data) = flip_in_events(&buf[..got as usize]) {
                return Ok(data);
            }
        }
    }

    /// Makes a `width` x `height` buffer of 32 bits a pixel.
    pub fn create_dumb(&self, width: u32, height: u32) -> Result<Dumb, String> {
        let mut d = CreateDumb {
            width,
            height,
            bpp: 32,
            ..CreateDumb::default()
        };
        self.ioctl("making a screen buffer", CREATE_DUMB, &mut d)?;
        Ok(Dumb {
            handle: d.handle,
            pitch: d.pitch,
            size: d.size,
            width,
            height,
        })
    }

    pub fn destroy_dumb(&self, handle: u32) -> Result<(), String> {
        let mut d = DestroyDumb { handle };
        self.ioctl("freeing a screen buffer", DESTROY_DUMB, &mut d)
    }

    /// A framebuffer (something a CRTC can show) over `dumb`, in `format`.
    pub fn add_fb(&self, dumb: &Dumb, format: u32) -> Result<u32, String> {
        let mut f = FbCmd2 {
            width: dumb.width,
            height: dumb.height,
            pixel_format: format,
            ..FbCmd2::default()
        };
        f.handles[0] = dumb.handle;
        f.pitches[0] = dumb.pitch;
        self.ioctl("making a framebuffer", ADD_FB2, &mut f)?;
        Ok(f.fb_id)
    }

    pub fn rm_fb(&self, fb: u32) -> Result<(), String> {
        let mut id = fb;
        self.ioctl("removing a framebuffer", RM_FB, &mut id)
    }

    /// The buffer as a dma-buf, for the GPU to draw into.
    pub fn dmabuf(&self, dumb: &Dumb) -> Result<OwnedFd, String> {
        let mut p = PrimeHandle {
            handle: dumb.handle,
            flags: PRIME_FLAGS,
            fd: -1,
        };
        self.ioctl("sharing a screen buffer", PRIME_HANDLE_TO_FD, &mut p)?;
        // SAFETY: a file descriptor the kernel just gave this process.
        Ok(unsafe { OwnedFd::from_raw_fd(p.fd as RawFd) })
    }

    /// Maps the buffer into this process (to check what the GPU drew).
    pub fn map(&self, dumb: &Dumb) -> Result<Mapping, String> {
        let mut m = MapDumb {
            handle: dumb.handle,
            ..MapDumb::default()
        };
        self.ioctl("mapping a screen buffer", MAP_DUMB, &mut m)?;
        let len = dumb.size as usize;
        // SAFETY: mapping `len` bytes of the card at the offset the kernel
        // gave for this buffer; checked for failure below.
        let ptr = unsafe {
            libc::mmap64(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                self.fd.as_raw_fd(),
                m.offset as libc::off64_t,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(format!("mapping a screen buffer: {}", errno()));
        }
        Ok(Mapping { ptr, len })
    }

    /// Becomes the card's master (needed to change what it shows). Fails
    /// while another program holds it.
    pub fn set_master(&self) -> Result<(), String> {
        self.ioctl_none("taking the screen (DRM master)", SET_MASTER)
    }

    pub fn drop_master(&self) -> Result<(), String> {
        self.ioctl_none("giving the screen back (DRM master)", DROP_MASTER)
    }
}

/// The user data of the first page-flip event in `bytes` (events as read
/// from the card: a type and a length each, then the rest).
pub(crate) fn flip_in_events(bytes: &[u8]) -> Option<u64> {
    let mut at = 0;
    while at + EVENT_HEAD <= bytes.len() {
        let kind = u32::from_ne_bytes(bytes[at..at + 4].try_into().ok()?);
        let len = u32::from_ne_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        if len < EVENT_HEAD || at + len > bytes.len() {
            return None;
        }
        if kind == EVENT_FLIP_COMPLETE && len >= 16 {
            return Some(u64::from_ne_bytes(bytes[at + 8..at + 16].try_into().ok()?));
        }
        at += len;
    }
    None
}

// ------------------------------------------------------------ the display

/// The screen that is lit now: its connector, the CRTC driving it, and
/// what that CRTC showed before we touched it (put back on drop).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Output {
    pub connector: u32,
    pub crtc: Crtc,
}

impl Output {
    /// The mode it runs (it is lit, so it has one).
    pub fn mode(&self) -> ModeInfo {
        self.crtc.mode.unwrap_or_default()
    }
}

/// Finds the one connected screen whose CRTC is lit. Reads only.
pub fn find_output(card: &Card) -> Result<Output, String> {
    let (_, connectors) = card.resources()?;
    let mut lit = Vec::new();
    for id in connectors {
        let (connected, encoder) = card.connector(id)?;
        if !connected || encoder == 0 {
            continue;
        }
        let crtc_id = card.encoder_crtc(encoder)?;
        if crtc_id == 0 {
            continue;
        }
        let crtc = card.crtc(crtc_id)?;
        if crtc.mode.is_some() && crtc.fb != 0 {
            lit.push(Output {
                connector: id,
                crtc,
            });
        }
    }
    match lit.len() {
        1 => Ok(lit[0]),
        0 => Err("no screen is lit on this card (nothing to take over)".into()),
        n => Err(format!(
            "{n} screens are lit on this card; this draws on exactly one"
        )),
    }
}

/// Buffers the GPU draws into and the panel shows in turn, through KMS.
pub struct KmsDisplay {
    card: Card,
    pub output: Output,
    bufs: Vec<(Dumb, u32)>,
    /// True once a buffer of ours is on the screen.
    lit: bool,
    /// A page flip was asked for and has not happened yet.
    pending: bool,
    flips: u64,
}

impl KmsDisplay {
    /// Opens `card`, finds the lit screen and makes `count` buffers of its
    /// size in XRGB8888. Changes nothing on the screen.
    pub fn open(card: &Path, count: usize) -> Result<KmsDisplay, String> {
        let card = Card::open(card)?;
        let output = find_output(&card)?;
        let mode = output.mode();
        let mut d = KmsDisplay {
            card,
            output,
            bufs: Vec::new(),
            lit: false,
            pending: false,
            flips: 0,
        };
        for _ in 0..count {
            let dumb = d
                .card
                .create_dumb(mode.hdisplay as u32, mode.vdisplay as u32)?;
            let fb = match d.card.add_fb(&dumb, XRGB8888) {
                Ok(fb) => fb,
                Err(e) => {
                    let _ = d.card.destroy_dumb(dumb.handle);
                    return Err(e);
                }
            };
            d.bufs.push((dumb, fb));
        }
        Ok(d)
    }

    /// The screen's size in its own pixels (800 x 1280 on the MPC One).
    pub fn size(&self) -> (i32, i32) {
        let m = self.output.mode();
        (m.hdisplay as i32, m.vdisplay as i32)
    }

    pub fn buffer(&self, i: usize) -> Dumb {
        self.bufs[i].0
    }

    pub fn dmabuf(&self, i: usize) -> Result<OwnedFd, String> {
        self.card.dmabuf(&self.bufs[i].0)
    }

    pub fn map(&self, i: usize) -> Result<Mapping, String> {
        self.card.map(&self.bufs[i].0)
    }

    /// Page flips done so far.
    pub fn flips(&self) -> u64 {
        self.flips
    }

    /// Puts the screen back as it was found (the framebuffer console's
    /// picture), if a buffer of ours is on it. Safe to call twice.
    pub fn restore(&mut self) -> Result<(), String> {
        if !self.lit {
            return Ok(());
        }
        if self.pending {
            let _ = self.card.wait_flip(FLIP_TIMEOUT);
            self.pending = false;
        }
        let c = self.output.crtc;
        let r = self.card.set_crtc(
            c.id,
            c.fb,
            c.x,
            c.y,
            self.output.connector,
            &self.output.mode(),
        );
        self.lit = false;
        let _ = self.card.drop_master();
        r
    }
}

impl Display for KmsDisplay {
    fn show(&mut self, i: usize) -> Result<(), String> {
        let fb = self.bufs[i].1;
        if !self.lit {
            // The first change to the screen: our buffer, in the mode the
            // CRTC already runs (no new timings for the panel).
            self.card.set_master()?;
            let c = self.output.crtc;
            self.card
                .set_crtc(c.id, fb, 0, 0, self.output.connector, &self.output.mode())?;
            self.lit = true;
            return Ok(());
        }
        self.card.page_flip(self.output.crtc.id, fb, i as u64)?;
        self.pending = true;
        Ok(())
    }

    fn wait(&mut self) -> Result<(), String> {
        if self.pending {
            self.card.wait_flip(FLIP_TIMEOUT)?;
            self.pending = false;
            self.flips += 1;
        }
        Ok(())
    }
}

impl Drop for KmsDisplay {
    fn drop(&mut self) {
        let _ = self.restore();
        for (dumb, fb) in self.bufs.drain(..) {
            let _ = self.card.rm_fb(fb);
            let _ = self.card.destroy_dumb(dumb.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sizes and request numbers from the kernel's headers (Linux 6.8's
    /// uapi, printed by a C program built for x86-64 and for 32-bit ARM
    /// hard float, which agreed on every one of these).
    #[test]
    fn the_structures_match_the_kernels() {
        assert_eq!(size_of::<CardRes>(), 64);
        assert_eq!(size_of::<GetConnector>(), 80);
        assert_eq!(size_of::<GetEncoder>(), 20);
        assert_eq!(size_of::<ModeCrtc>(), 104);
        assert_eq!(size_of::<ModeInfo>(), 68);
        assert_eq!(size_of::<CreateDumb>(), 32);
        assert_eq!(size_of::<MapDumb>(), 16);
        assert_eq!(size_of::<DestroyDumb>(), 4);
        assert_eq!(size_of::<FbCmd2>(), 104);
        assert_eq!(size_of::<PrimeHandle>(), 12);
        assert_eq!(size_of::<PageFlip>(), 24);
        assert_eq!(std::mem::offset_of!(GetConnector, count_modes), 32);
        assert_eq!(std::mem::offset_of!(GetConnector, encoder_id), 44);
        assert_eq!(std::mem::offset_of!(GetConnector, connection), 60);
        assert_eq!(std::mem::offset_of!(ModeCrtc, mode_valid), 32);
        assert_eq!(std::mem::offset_of!(ModeCrtc, mode), 36);
        assert_eq!(std::mem::offset_of!(FbCmd2, handles), 20);
        assert_eq!(std::mem::offset_of!(FbCmd2, pitches), 36);
        assert_eq!(std::mem::offset_of!(FbCmd2, modifier), 72);
        assert_eq!(std::mem::offset_of!(ModeInfo, vrefresh), 24);
        assert_eq!(std::mem::offset_of!(ModeInfo, name), 36);
        assert_eq!(std::mem::offset_of!(CreateDumb, handle), 16);
        assert_eq!(std::mem::offset_of!(CreateDumb, size), 24);
        assert_eq!(GET_RESOURCES, 0xc04064a0);
        assert_eq!(GET_CONNECTOR, 0xc05064a7);
        assert_eq!(GET_ENCODER, 0xc01464a6);
        assert_eq!(GET_CRTC, 0xc06864a1);
        assert_eq!(SET_CRTC, 0xc06864a2);
        assert_eq!(CREATE_DUMB, 0xc02064b2);
        assert_eq!(MAP_DUMB, 0xc01064b3);
        assert_eq!(DESTROY_DUMB, 0xc00464b4);
        assert_eq!(ADD_FB2, 0xc06864b8);
        assert_eq!(RM_FB, 0xc00464af);
        assert_eq!(PRIME_HANDLE_TO_FD, 0xc00c642d);
        assert_eq!(PAGE_FLIP, 0xc01864b0);
        assert_eq!(SET_MASTER, 0x641e);
        assert_eq!(DROP_MASTER, 0x641f);
        assert_eq!(PRIME_FLAGS, 0o2000000 | 2);
        assert_eq!(XRGB8888, 0x34325258);
    }

    #[test]
    fn a_flip_is_found_among_other_events() {
        let mut b = Vec::new();
        // A vblank event (type 1), then a flip complete with user data 7.
        for (kind, data) in [(1u32, 99u64), (EVENT_FLIP_COMPLETE, 7)] {
            b.extend_from_slice(&kind.to_ne_bytes());
            b.extend_from_slice(&32u32.to_ne_bytes());
            b.extend_from_slice(&data.to_ne_bytes());
            b.extend_from_slice(&[0; 16]);
        }
        assert_eq!(flip_in_events(&b), Some(7));
        assert_eq!(flip_in_events(&b[..32]), None);
        // A length that runs past the end is not trusted.
        let mut bad = b[32..].to_vec();
        bad[4..8].copy_from_slice(&999u32.to_ne_bytes());
        assert_eq!(flip_in_events(&bad), None);
    }

    #[test]
    fn a_card_that_is_not_there_says_so() {
        let e = KmsDisplay::open(Path::new("/nonexistent/dri/card9"), 2)
            .err()
            .expect("no card");
        assert!(e.contains("/nonexistent/dri/card9"), "{e}");
    }
}
