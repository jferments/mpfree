//! A canvas that records a frame's drawing calls as a short list (a display
//! list), and plays the list back onto any other canvas.
//!
//! This is the seam between the screens and whatever puts their pixels on
//! the panel. Every widget draws through [`Canvas`]'s calls; the
//! recorder keeps them as [`Op`]s, and [`Recorder::replay`] makes the same
//! calls on another canvas: [`crate::SoftCanvas`] today, a GPU renderer
//! next (the performance plan's "GPU drawing and compute"). Played back on
//! a `SoftCanvas` the picture is the same, bit for bit, as drawing straight
//! into it.
//!
//! Masks (text), shapes (curves and knobs: [`Shape`], kept as their parts)
//! and images are kept once each, by content, in a [`Store`]: a frame's list names them by id, so a renderer in another
//! process receives each glyph once, not every frame. Entries no frame has
//! used for a while are dropped once the store passes its byte budget.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::canvas::{Canvas, Image};
use crate::color::Color;
use crate::geom::Rect;
use crate::shape::{Part, Shape};

/// One drawing call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// [`Canvas::set_clip`] with this rectangle (as asked; cut on playback).
    Clip(Rect),
    Fill(Rect, Color),
    FillRound {
        r: Rect,
        radius: i32,
        c: Color,
    },
    StrokeRound {
        r: Rect,
        radius: i32,
        width: i32,
        c: Color,
    },
    /// [`Canvas::blend_mask`] with the stored mask `id`.
    Mask {
        x: i32,
        y: i32,
        id: u32,
        c: Color,
    },
    /// [`Canvas::draw_image`] with the stored image `id`.
    Image {
        x: i32,
        y: i32,
        id: u32,
    },
    /// [`Canvas::blend_shape`] with the stored shape `id` (it carries its
    /// own rectangle).
    Shape {
        id: u32,
        c: Color,
    },
}

/// A coverage mask: one byte per pixel, `w` x `h`, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

struct Entry<T> {
    item: T,
    hash: u64,
    bytes: usize,
    /// The last frame that used it.
    used: u64,
}

/// Masks and images by content, each kept once, named by an id that is
/// never reused.
pub struct Store {
    masks: HashMap<u32, Entry<Mask>>,
    images: HashMap<u32, Entry<Image>>,
    shapes: HashMap<u32, Entry<Shape>>,
    by_hash: HashMap<u64, Vec<u32>>,
    next: u32,
    bytes: usize,
    budget: usize,
    /// Ids added and dropped since [`Store::take_changes`], for a renderer
    /// that keeps its own copies.
    added: Vec<u32>,
    dropped: Vec<u32>,
}

/// What a frame's list cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Drawing calls in the list.
    pub ops: usize,
    /// Mask, image and shape calls, of those.
    pub masks: usize,
    pub images: usize,
    pub shapes: usize,
    /// Masks and images the store had not seen before, and their bytes.
    pub new_items: usize,
    pub new_bytes: usize,
    /// The list's own bytes (without the new items).
    pub list_bytes: usize,
}

impl FrameStats {
    /// What a frame sends to a renderer that keeps its own copies: the
    /// list and the new items.
    pub fn sent_bytes(&self) -> usize {
        self.list_bytes + self.new_bytes
    }
}

/// The store's default budget: past it, entries no frame used lately go.
pub const STORE_BUDGET: usize = 8 << 20;

/// Frames an entry is kept unused, at least, before it may be dropped.
const KEEP_FRAMES: u64 = 2;

fn content_hash(kind: u8, w: usize, h: usize, bytes: impl Hash) -> u64 {
    let mut s = DefaultHasher::new();
    (kind, w, h).hash(&mut s);
    bytes.hash(&mut s);
    s.finish()
}

/// A shape's hash: its rectangle and every number of its parts, bit for bit.
fn shape_hash(shape: &Shape) -> u64 {
    let mut s = DefaultHasher::new();
    (2u8, shape.r.x, shape.r.y, shape.r.w, shape.r.h).hash(&mut s);
    for p in &shape.parts {
        let (tag, v) = p.numbers();
        tag.hash(&mut s);
        for x in v {
            x.to_bits().hash(&mut s);
        }
    }
    s.finish()
}

/// What a shape costs the store: its parts, as the wire carries them.
fn shape_bytes(shape: &Shape) -> usize {
    shape
        .parts
        .iter()
        .map(|p| 1 + 4 * p.numbers().1.len())
        .sum::<usize>()
        + 24
}

impl Store {
    pub fn new(budget: usize) -> Store {
        Store {
            masks: HashMap::new(),
            images: HashMap::new(),
            shapes: HashMap::new(),
            by_hash: HashMap::new(),
            next: 0,
            bytes: 0,
            budget,
            added: Vec::new(),
            dropped: Vec::new(),
        }
    }

    pub fn mask(&self, id: u32) -> Option<&Mask> {
        self.masks.get(&id).map(|e| &e.item)
    }

    pub fn image(&self, id: u32) -> Option<&Image> {
        self.images.get(&id).map(|e| &e.item)
    }

    pub fn shape(&self, id: u32) -> Option<&Shape> {
        self.shapes.get(&id).map(|e| &e.item)
    }

    /// Bytes held now.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.masks.len() + self.images.len() + self.shapes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The ids added and dropped since the last call.
    pub fn take_changes(&mut self) -> (Vec<u32>, Vec<u32>) {
        (
            std::mem::take(&mut self.added),
            std::mem::take(&mut self.dropped),
        )
    }

    /// The mask's id, and whether it is new.
    fn intern_mask(&mut self, w: usize, h: usize, data: &[u8], frame: u64) -> (u32, bool) {
        let hash = content_hash(0, w, h, data);
        if let Some(ids) = self.by_hash.get(&hash) {
            for id in ids {
                if let Some(e) = self.masks.get_mut(id) {
                    if e.item.w == w && e.item.h == h && e.item.data == data {
                        e.used = frame;
                        return (*id, false);
                    }
                }
            }
        }
        let id = self.fresh_id(hash);
        let bytes = data.len();
        self.bytes += bytes;
        self.masks.insert(
            id,
            Entry {
                item: Mask {
                    w,
                    h,
                    data: data.to_vec(),
                },
                hash,
                bytes,
                used: frame,
            },
        );
        (id, true)
    }

    fn intern_image(&mut self, img: &Image, frame: u64) -> (u32, bool) {
        let hash = content_hash(1, img.w as usize, img.h as usize, &img.px);
        if let Some(ids) = self.by_hash.get(&hash) {
            for id in ids {
                if let Some(e) = self.images.get_mut(id) {
                    if e.item == *img {
                        e.used = frame;
                        return (*id, false);
                    }
                }
            }
        }
        let id = self.fresh_id(hash);
        let bytes = img.px.len() * 4;
        self.bytes += bytes;
        self.images.insert(
            id,
            Entry {
                item: img.clone(),
                hash,
                bytes,
                used: frame,
            },
        );
        (id, true)
    }

    fn intern_shape(&mut self, shape: &Shape, frame: u64) -> (u32, bool) {
        let hash = shape_hash(shape);
        if let Some(ids) = self.by_hash.get(&hash) {
            for id in ids {
                if let Some(e) = self.shapes.get_mut(id) {
                    if e.item == *shape {
                        e.used = frame;
                        return (*id, false);
                    }
                }
            }
        }
        let id = self.fresh_id(hash);
        let bytes = shape_bytes(shape);
        self.bytes += bytes;
        self.shapes.insert(
            id,
            Entry {
                item: shape.clone(),
                hash,
                bytes,
                used: frame,
            },
        );
        (id, true)
    }

    fn fresh_id(&mut self, hash: u64) -> u32 {
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        self.by_hash.entry(hash).or_default().push(id);
        self.added.push(id);
        id
    }

    /// Past the budget, drops the entries unused for the longest, never one
    /// used in the last few frames.
    fn trim(&mut self, frame: u64) {
        if self.bytes <= self.budget {
            return;
        }
        let mut old: Vec<(u64, u32)> = self
            .masks
            .iter()
            .map(|(id, e)| (e.used, *id))
            .chain(self.images.iter().map(|(id, e)| (e.used, *id)))
            .chain(self.shapes.iter().map(|(id, e)| (e.used, *id)))
            .filter(|(used, _)| used + KEEP_FRAMES <= frame)
            .collect();
        old.sort_unstable();
        for (_, id) in old {
            if self.bytes <= self.budget {
                break;
            }
            let (hash, bytes) = if let Some(e) = self.masks.remove(&id) {
                (e.hash, e.bytes)
            } else if let Some(e) = self.images.remove(&id) {
                (e.hash, e.bytes)
            } else if let Some(e) = self.shapes.remove(&id) {
                (e.hash, e.bytes)
            } else {
                continue;
            };
            self.bytes -= bytes;
            if let Some(ids) = self.by_hash.get_mut(&hash) {
                ids.retain(|i| *i != id);
                if ids.is_empty() {
                    self.by_hash.remove(&hash);
                }
            }
            self.dropped.push(id);
        }
    }
}

/// A canvas of `w` x `h` that keeps each frame's calls.
pub struct Recorder {
    w: i32,
    h: i32,
    clip: Rect,
    ops: Vec<Op>,
    store: Store,
    frame: u64,
    stats: FrameStats,
}

impl Recorder {
    pub fn new(w: i32, h: i32) -> Recorder {
        Recorder::with_budget(w, h, STORE_BUDGET)
    }

    pub fn with_budget(w: i32, h: i32, budget: usize) -> Recorder {
        Recorder {
            w,
            h,
            clip: Rect::new(0, 0, w, h),
            ops: Vec::new(),
            store: Store::new(budget),
            frame: 0,
            stats: FrameStats::default(),
        }
    }

    /// Starts a new frame: the list empties (its memory is kept), the clip
    /// is the whole canvas again, and the store may drop old entries.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        self.store.trim(self.frame);
        self.ops.clear();
        self.clip = Rect::new(0, 0, self.w, self.h);
        self.stats = FrameStats::default();
    }

    /// The frame's calls, in order.
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    /// What this frame's list cost so far.
    pub fn stats(&self) -> FrameStats {
        FrameStats {
            ops: self.ops.len(),
            list_bytes: self.ops.len() * std::mem::size_of::<Op>(),
            ..self.stats
        }
    }

    /// Makes this frame's calls on `canvas`, in order. The clip is left as
    /// the list's last one.
    pub fn replay(&self, canvas: &mut dyn Canvas) {
        replay(&self.ops, &self.store, canvas);
    }

    fn push(&mut self, op: Op) {
        self.ops.push(op);
    }
}

/// Makes the calls in `ops` on `canvas`, with masks and images from
/// `store`. An id the store no longer holds is skipped.
pub fn replay(ops: &[Op], store: &Store, canvas: &mut dyn Canvas) {
    for op in ops {
        match *op {
            Op::Clip(r) => canvas.set_clip(r),
            Op::Fill(r, c) => canvas.fill_rect(r, c),
            Op::FillRound { r, radius, c } => canvas.fill_round_rect(r, radius, c),
            Op::StrokeRound {
                r,
                radius,
                width,
                c,
            } => canvas.stroke_round_rect(r, radius, width, c),
            Op::Mask { x, y, id, c } => {
                if let Some(m) = store.mask(id) {
                    canvas.blend_mask(x, y, m.w, m.h, &m.data, c);
                }
            }
            Op::Image { x, y, id } => {
                if let Some(img) = store.image(id) {
                    canvas.draw_image(x, y, img);
                }
            }
            Op::Shape { id, c } => {
                if let Some(shape) = store.shape(id) {
                    canvas.blend_shape(shape, c);
                }
            }
        }
    }
}

impl Canvas for Recorder {
    fn size(&self) -> (i32, i32) {
        (self.w, self.h)
    }

    fn clip(&self) -> Rect {
        self.clip
    }

    fn set_clip(&mut self, r: Rect) {
        self.clip = r.intersect(Rect::new(0, 0, self.w, self.h));
        self.push(Op::Clip(r));
    }

    fn fill_rect(&mut self, r: Rect, c: Color) {
        self.push(Op::Fill(r, c));
    }

    fn fill_round_rect(&mut self, r: Rect, radius: i32, c: Color) {
        self.push(Op::FillRound { r, radius, c });
    }

    fn stroke_round_rect(&mut self, r: Rect, radius: i32, width: i32, c: Color) {
        self.push(Op::StrokeRound {
            r,
            radius,
            width,
            c,
        });
    }

    fn blend_mask(&mut self, x: i32, y: i32, w: usize, h: usize, mask: &[u8], c: Color) {
        // The same bytes the software canvas reads (it reads w * h).
        let data = &mask[..(w * h).min(mask.len())];
        let (id, new) = self.store.intern_mask(w, h, data, self.frame);
        self.stats.masks += 1;
        if new {
            self.stats.new_items += 1;
            self.stats.new_bytes += data.len();
        }
        self.push(Op::Mask { x, y, id, c });
    }

    fn draw_image(&mut self, x: i32, y: i32, img: &Image) {
        let (id, new) = self.store.intern_image(img, self.frame);
        self.stats.images += 1;
        if new {
            self.stats.new_items += 1;
            self.stats.new_bytes += img.px.len() * 4;
        }
        self.push(Op::Image { x, y, id });
    }

    fn blend_shape(&mut self, shape: &Shape, c: Color) {
        if shape.r.is_empty() {
            return;
        }
        let (id, new) = self.store.intern_shape(shape, self.frame);
        self.stats.shapes += 1;
        if new {
            self.stats.new_items += 1;
            self.stats.new_bytes += shape_bytes(shape);
        }
        self.push(Op::Shape { id, c });
    }
}

// ------------------------------------------------------------ the wire format

/// The first bytes of every encoded frame.
const WIRE_MAGIC: [u8; 4] = *b"MPDL";
/// 2: shapes (item kind 2, call 6).
const WIRE_VERSION: u16 = 2;
/// Header: magic, version, 2 spare bytes, w, h, the frame's length.
const WIRE_HEADER: usize = 4 + 2 + 2 + 4 + 4 + 4;
/// The most pixels one mask or image may have (a 4096 x 4096 picture), so
/// junk cannot ask for gigabytes.
const WIRE_MAX_PIXELS: usize = 4096 * 4096;

/// A mask, an image or a shape as a frame carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Mask(Mask),
    Image(Image),
    Shape(Shape),
}

/// One frame as a renderer in another process receives it: the store's
/// changes since the last frame, then the list. [`Store::apply`] keeps a
/// copy of the sender's store up to date from it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// The canvas size.
    pub w: i32,
    pub h: i32,
    /// Ids the sender's store let go.
    pub dropped: Vec<u32>,
    /// New masks, images and shapes, with their ids.
    pub added: Vec<(u32, Item)>,
    pub ops: Vec<Op>,
}

impl Recorder {
    /// Appends this frame to `out` in the wire format: the store's changes
    /// since the last call (it calls [`Store::take_changes`]) and the list.
    pub fn encode_frame(&mut self, out: &mut Vec<u8>) {
        let (added, dropped) = self.store.take_changes();
        encode_frame(
            self.w,
            self.h,
            &self.ops,
            &self.store,
            &added,
            &dropped,
            out,
        );
    }
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_rect(out: &mut Vec<u8>, r: Rect) {
    for v in [r.x, r.y, r.w, r.h] {
        put_i32(out, v);
    }
}

fn put_color(out: &mut Vec<u8>, c: Color) {
    out.extend_from_slice(&[c.r, c.g, c.b, c.a]);
}

fn put_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// An item of the store, as `encode_frame` finds it.
enum ItemRef<'a> {
    Mask(&'a Mask),
    Image(&'a Image),
    Shape(&'a Shape),
}

/// Appends one frame to `out`, all numbers little-endian. An id in `added`
/// that `store` no longer holds (added and dropped since the last frame) is
/// left out; it is in `dropped` too.
pub fn encode_frame(
    w: i32,
    h: i32,
    ops: &[Op],
    store: &Store,
    added: &[u32],
    dropped: &[u32],
    out: &mut Vec<u8>,
) {
    let start = out.len();
    out.extend_from_slice(&WIRE_MAGIC);
    out.extend_from_slice(&WIRE_VERSION.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    put_i32(out, w);
    put_i32(out, h);
    put_u32(out, 0); // the length, filled in at the end
    put_u32(out, dropped.len() as u32);
    for id in dropped {
        put_u32(out, *id);
    }
    let items: Vec<(u32, ItemRef)> = added
        .iter()
        .filter_map(|id| {
            let item = if let Some(m) = store.mask(*id) {
                ItemRef::Mask(m)
            } else if let Some(i) = store.image(*id) {
                ItemRef::Image(i)
            } else {
                ItemRef::Shape(store.shape(*id)?)
            };
            Some((*id, item))
        })
        .collect();
    put_u32(out, items.len() as u32);
    for (id, item) in items {
        put_u32(out, id);
        match item {
            ItemRef::Mask(m) => {
                out.push(0);
                put_u32(out, m.w as u32);
                put_u32(out, m.h as u32);
                out.extend_from_slice(&m.data);
            }
            ItemRef::Image(img) => {
                out.push(1);
                put_u32(out, img.w as u32);
                put_u32(out, img.h as u32);
                for p in &img.px {
                    put_u32(out, *p);
                }
            }
            ItemRef::Shape(sh) => {
                // Its size first, like the others, then where it is and
                // its parts: a tag and the part's numbers each.
                out.push(2);
                put_u32(out, sh.r.w as u32);
                put_u32(out, sh.r.h as u32);
                put_i32(out, sh.r.x);
                put_i32(out, sh.r.y);
                put_u32(out, sh.parts.len() as u32);
                for p in &sh.parts {
                    let (tag, v) = p.numbers();
                    out.push(tag);
                    for x in v {
                        put_f32(out, x);
                    }
                }
            }
        }
    }
    put_u32(out, ops.len() as u32);
    for op in ops {
        match *op {
            Op::Clip(r) => {
                out.push(0);
                put_rect(out, r);
            }
            Op::Fill(r, c) => {
                out.push(1);
                put_rect(out, r);
                put_color(out, c);
            }
            Op::FillRound { r, radius, c } => {
                out.push(2);
                put_rect(out, r);
                put_i32(out, radius);
                put_color(out, c);
            }
            Op::StrokeRound {
                r,
                radius,
                width,
                c,
            } => {
                out.push(3);
                put_rect(out, r);
                put_i32(out, radius);
                put_i32(out, width);
                put_color(out, c);
            }
            Op::Mask { x, y, id, c } => {
                out.push(4);
                put_i32(out, x);
                put_i32(out, y);
                put_u32(out, id);
                put_color(out, c);
            }
            Op::Image { x, y, id } => {
                out.push(5);
                put_i32(out, x);
                put_i32(out, y);
                put_u32(out, id);
            }
            Op::Shape { id, c } => {
                out.push(6);
                put_u32(out, id);
                put_color(out, c);
            }
        }
    }
    let len = (out.len() - start) as u32;
    out[start + 16..start + 20].copy_from_slice(&len.to_le_bytes());
}

/// Reads the wire format, refusing (never panicking on) anything short,
/// oversized or unknown.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(n)
            .filter(|e| *e <= self.b.len())
            .ok_or_else(|| format!("the frame ends early, at byte {}", self.at))?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn i32(&mut self) -> Result<i32, String> {
        Ok(self.u32()? as i32)
    }

    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn rect(&mut self) -> Result<Rect, String> {
        Ok(Rect::new(
            self.i32()?,
            self.i32()?,
            self.i32()?,
            self.i32()?,
        ))
    }

    fn color(&mut self) -> Result<Color, String> {
        let s = self.take(4)?;
        Ok(Color {
            r: s[0],
            g: s[1],
            b: s[2],
            a: s[3],
        })
    }

    /// A count of things at least `each` bytes long, checked against what
    /// is left so junk cannot reserve memory it does not carry.
    fn count(&mut self, each: usize) -> Result<usize, String> {
        let n = self.u32()? as usize;
        if n.saturating_mul(each) > self.b.len() - self.at {
            return Err(format!("a count of {n} is more than the frame holds"));
        }
        Ok(n)
    }

    fn size(&mut self) -> Result<(usize, usize), String> {
        let (w, h) = (self.u32()? as usize, self.u32()? as usize);
        if w.saturating_mul(h) > WIRE_MAX_PIXELS || w > i32::MAX as usize || h > i32::MAX as usize {
            return Err(format!("a {w} x {h} item is too big"));
        }
        Ok((w, h))
    }
}

impl Frame {
    /// Reads one frame from the start of `bytes`, and how many bytes it
    /// took (frames can follow each other, as in a file).
    pub fn decode(bytes: &[u8]) -> Result<(Frame, usize), String> {
        let mut r = Reader { b: bytes, at: 0 };
        if r.take(4)? != WIRE_MAGIC {
            return Err("not a display list (no MPDL at the start)".into());
        }
        let v = r.take(2)?;
        let version = u16::from_le_bytes([v[0], v[1]]);
        if version != WIRE_VERSION {
            return Err(format!(
                "display list version {version}; this build reads {WIRE_VERSION}"
            ));
        }
        r.take(2)?;
        let (w, h) = (r.i32()?, r.i32()?);
        let len = r.u32()? as usize;
        if len < WIRE_HEADER || len > bytes.len() {
            return Err(format!(
                "the frame says it is {len} bytes; {} are here",
                bytes.len()
            ));
        }
        // Read only this frame's bytes from here on.
        r.b = &bytes[..len];
        let mut f = Frame {
            w,
            h,
            ..Frame::default()
        };
        for _ in 0..r.count(4)? {
            f.dropped.push(r.u32()?);
        }
        for _ in 0..r.count(13)? {
            let id = r.u32()?;
            let kind = r.u8()?;
            let (w, h) = r.size()?;
            let item = match kind {
                0 => Item::Mask(Mask {
                    w,
                    h,
                    data: r.take(w * h)?.to_vec(),
                }),
                1 => {
                    let raw = r.take(w * h * 4)?;
                    let px = raw
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|p| u32::from_le_bytes(*p))
                        .collect();
                    Item::Image(Image {
                        w: w as i32,
                        h: h as i32,
                        px,
                    })
                }
                2 => {
                    let (x, y) = (r.i32()?, r.i32()?);
                    let mut shape = Shape::new(Rect::new(x, y, w as i32, h as i32));
                    for _ in 0..r.count(13)? {
                        let tag = r.u8()?;
                        let n = Part::count_for(tag)
                            .ok_or_else(|| format!("unknown shape part {tag}"))?;
                        let mut v = [0f32; 6];
                        for x in v.iter_mut().take(n) {
                            *x = r.f32()?;
                        }
                        shape
                            .parts
                            .push(Part::from_numbers(tag, &v[..n]).expect("counted above"));
                    }
                    Item::Shape(shape)
                }
                k => return Err(format!("unknown item kind {k}")),
            };
            f.added.push((id, item));
        }
        for _ in 0..r.count(13)? {
            let op = match r.u8()? {
                0 => Op::Clip(r.rect()?),
                1 => Op::Fill(r.rect()?, r.color()?),
                2 => Op::FillRound {
                    r: r.rect()?,
                    radius: r.i32()?,
                    c: r.color()?,
                },
                3 => Op::StrokeRound {
                    r: r.rect()?,
                    radius: r.i32()?,
                    width: r.i32()?,
                    c: r.color()?,
                },
                4 => Op::Mask {
                    x: r.i32()?,
                    y: r.i32()?,
                    id: r.u32()?,
                    c: r.color()?,
                },
                5 => Op::Image {
                    x: r.i32()?,
                    y: r.i32()?,
                    id: r.u32()?,
                },
                6 => Op::Shape {
                    id: r.u32()?,
                    c: r.color()?,
                },
                t => return Err(format!("unknown drawing call {t}")),
            };
            f.ops.push(op);
        }
        if r.at != len {
            return Err(format!("{} bytes left over in the frame", len - r.at));
        }
        Ok((f, len))
    }

    /// The frames one after another in `bytes` (a file of them).
    pub fn decode_all(mut bytes: &[u8]) -> Result<Vec<Frame>, String> {
        let mut frames = Vec::new();
        while !bytes.is_empty() {
            let (f, n) = Frame::decode(bytes)?;
            frames.push(f);
            bytes = &bytes[n..];
        }
        Ok(frames)
    }
}

impl Store {
    /// Brings a copy of the sender's store up to date with a received frame:
    /// drops what the sender dropped and keeps the new items under the
    /// sender's ids. For the receiving side only: the copy never interns.
    pub fn apply(&mut self, frame: &Frame) {
        for id in &frame.dropped {
            if let Some(e) = self.masks.remove(id) {
                self.bytes -= e.bytes;
            }
            if let Some(e) = self.images.remove(id) {
                self.bytes -= e.bytes;
            }
            if let Some(e) = self.shapes.remove(id) {
                self.bytes -= e.bytes;
            }
        }
        for (id, item) in &frame.added {
            self.next = self.next.max(id.wrapping_add(1));
            match item {
                Item::Mask(m) => {
                    let bytes = m.data.len();
                    self.bytes += bytes;
                    if let Some(old) = self.masks.insert(
                        *id,
                        Entry {
                            item: m.clone(),
                            hash: 0,
                            bytes,
                            used: 0,
                        },
                    ) {
                        self.bytes -= old.bytes;
                    }
                }
                Item::Image(img) => {
                    let bytes = img.px.len() * 4;
                    self.bytes += bytes;
                    if let Some(old) = self.images.insert(
                        *id,
                        Entry {
                            item: img.clone(),
                            hash: 0,
                            bytes,
                            used: 0,
                        },
                    ) {
                        self.bytes -= old.bytes;
                    }
                }
                Item::Shape(sh) => {
                    let bytes = shape_bytes(sh);
                    self.bytes += bytes;
                    if let Some(old) = self.shapes.insert(
                        *id,
                        Entry {
                            item: sh.clone(),
                            hash: 0,
                            bytes,
                            used: 0,
                        },
                    ) {
                        self.bytes -= old.bytes;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::SoftCanvas;

    const W: i32 = 64;
    const H: i32 = 40;

    /// A little of everything, with clips, overlaps and half-covered masks.
    fn draw(c: &mut dyn Canvas, k: u8) {
        c.fill_rect(Rect::new(0, 0, W, H), Color::rgb(0x0A0C14));
        c.fill_round_rect(Rect::new(3, 4, 30, 20), 6, Color::rgb(0xC82800 | k as u32));
        c.set_clip(Rect::new(10, 2, 40, 30));
        c.stroke_round_rect(
            Rect::new(5, 5, 50, 30),
            8,
            2,
            Color::rgb(0x5AC878).with_alpha(180),
        );
        let mask: Vec<u8> = (0..12 * 9).map(|i| (i * 7 % 256) as u8).collect();
        c.blend_mask(20, 10, 12, 9, &mask, Color::rgb(0xFFFFFF));
        c.blend_mask(-4, 30, 12, 9, &mask, Color::rgb(0xFF8000).with_alpha(200));
        c.set_clip(Rect::new(-10, -10, 500, 500));
        let mut img = Image::new(8, 6);
        for (i, p) in img.px.iter_mut().enumerate() {
            *p = (i as u32 * 0x010203) ^ k as u32;
        }
        c.draw_image(50, 30, &img);
        c.fill_rect(
            Rect::new(60, 35, 20, 20),
            Color::rgb(0x0000FF).with_alpha(100),
        );
    }

    #[test]
    fn played_back_it_is_the_same_picture_bit_for_bit() {
        let mut direct = SoftCanvas::new(W, H);
        draw(&mut direct, 7);
        let mut rec = Recorder::new(W, H);
        rec.begin_frame();
        draw(&mut rec, 7);
        let mut played = SoftCanvas::new(W, H);
        rec.replay(&mut played);
        assert_eq!(direct.px, played.px);
        assert_eq!(rec.clip(), direct.clip(), "the clip is tracked the same");
    }

    #[test]
    fn each_mask_and_image_is_kept_once() {
        let mut rec = Recorder::new(W, H);
        rec.begin_frame();
        draw(&mut rec, 7);
        let first = rec.stats();
        assert_eq!((first.masks, first.images), (2, 1));
        assert_eq!(first.new_items, 2, "the same mask twice is one entry");
        assert_eq!(first.new_bytes, 12 * 9 + 8 * 6 * 4);
        assert_eq!(first.ops, 9);
        let (added, dropped) = rec.store_mut().take_changes();
        assert_eq!((added.len(), dropped.len()), (2, 0));
        // The next frame draws the same: nothing new to send but the list.
        rec.begin_frame();
        draw(&mut rec, 7);
        let again = rec.stats();
        assert_eq!(again.new_items, 0);
        assert_eq!(again.sent_bytes(), again.list_bytes);
        // A changed image is a new entry; the mask is not.
        rec.begin_frame();
        draw(&mut rec, 8);
        assert_eq!(rec.stats().new_items, 1);
        assert_eq!(rec.store().len(), 3);
    }

    #[test]
    fn past_its_budget_the_store_drops_what_no_frame_uses() {
        let mut rec = Recorder::with_budget(W, H, 1000);
        let mut frames = Vec::new();
        for k in 0..6u8 {
            rec.begin_frame();
            let mask = vec![k; 400];
            rec.blend_mask(0, 0, 20, 20, &mask, Color::rgb(0x010203));
            let mut played = SoftCanvas::new(W, H);
            rec.replay(&mut played);
            frames.push(played.px);
        }
        assert!(rec.store().bytes() <= 1200, "{}", rec.store().bytes());
        let (added, dropped) = rec.store_mut().take_changes();
        assert_eq!(added.len(), 6);
        assert!(!dropped.is_empty());
        // The current frame's mask is still there, and plays back.
        assert!(rec.store().mask(*added.last().unwrap()).is_some());
        let mut direct = SoftCanvas::new(W, H);
        direct.blend_mask(0, 0, 20, 20, &[5; 400], Color::rgb(0x010203));
        assert_eq!(frames.last().unwrap(), &direct.px);
    }

    #[test]
    fn sent_as_bytes_a_frame_plays_back_the_same_from_the_copy() {
        // Frames sent one after another, as to a renderer in another
        // process, with a store small enough to drop entries on the way.
        let mut rec = Recorder::with_budget(W, H, 600);
        let mut copy = Store::new(usize::MAX);
        let mut wire = Vec::new();
        let mut direct = SoftCanvas::new(W, H);
        let mut played = SoftCanvas::new(W, H);
        for k in 0..5u8 {
            rec.begin_frame();
            draw(&mut rec, k);
            let mask = vec![k * 40; 300];
            rec.blend_mask(1, 1, 20, 15, &mask, Color::rgb(0x102030));
            draw(&mut direct, k);
            direct.blend_mask(1, 1, 20, 15, &mask, Color::rgb(0x102030));
            let before = wire.len();
            rec.encode_frame(&mut wire);
            let (frame, n) = Frame::decode(&wire[before..]).unwrap();
            assert_eq!(n, wire.len() - before);
            assert_eq!((frame.w, frame.h), (W, H));
            assert_eq!(frame.ops, rec.ops());
            copy.apply(&frame);
            replay(&frame.ops, &copy, &mut played);
            assert_eq!(direct.px, played.px, "frame {k}");
        }
        // The copy holds what the sender holds, no more.
        assert_eq!(copy.len(), rec.store().len());
        assert_eq!(copy.bytes(), rec.store().bytes());
        // A file of frames reads back as the same frames.
        let all = Frame::decode_all(&wire).unwrap();
        assert_eq!(all.len(), 5);
        assert!(
            all.iter().any(|f| !f.dropped.is_empty()),
            "something dropped"
        );
    }

    /// Shapes: a curve with the area under it, a knob ring, a dot and a
    /// dashed line, one cut by the clip.
    fn draw_shapes(c: &mut dyn Canvas, k: u8) {
        c.fill_rect(Rect::new(0, 0, W, H), Color::rgb(0x0A0C14));
        let mut s = Shape::new(Rect::new(2, 2, 40, 30));
        let pts: Vec<(f32, f32)> = (0..=20)
            .map(|i| {
                (
                    2.0 + i as f32 * 2.0,
                    16.0 + ((i as f32 + k as f32) * 0.7).sin() * 10.0,
                )
            })
            .collect();
        s.fill_under(&pts, 30.0, 0.4);
        s.polyline(&pts, 1.5);
        s.disc(20.5, 16.0, 3.0);
        s.draw(c, Color::rgb(0x40C0FF));
        c.set_clip(Rect::new(30, 0, 30, 40));
        let mut ring = Shape::new(Rect::new(28, 4, 34, 34));
        ring.arc(45.0, 21.0, 12.0, 2.4, 6.0 + k as f32 * 0.1, 3.0);
        ring.dashed(&[(30.0, 36.0), (60.0, 6.0)], 1.0, 4.0);
        ring.draw(c, Color::rgb(0xF0A040).with_alpha(200));
    }

    #[test]
    fn shapes_play_back_and_travel_as_their_parts() {
        let mut direct = SoftCanvas::new(W, H);
        draw_shapes(&mut direct, 1);
        let mut rec = Recorder::new(W, H);
        rec.begin_frame();
        draw_shapes(&mut rec, 1);
        let st = rec.stats();
        assert_eq!((st.shapes, st.masks, st.new_items), (2, 0, 2));
        assert!(
            st.new_bytes < 2000,
            "parts, not pixels: {} bytes",
            st.new_bytes
        );
        let mut played = SoftCanvas::new(W, H);
        rec.replay(&mut played);
        assert_eq!(direct.px, played.px, "the same picture, bit for bit");
        // Sent and played from the copy, frame after frame; a shape that
        // did not change is not sent again.
        let mut copy = Store::new(usize::MAX);
        let mut wire = Vec::new();
        rec.encode_frame(&mut wire);
        let (frame, _) = Frame::decode(&wire).unwrap();
        assert_eq!(
            frame
                .added
                .iter()
                .filter(|(_, it)| matches!(it, Item::Shape(_)))
                .count(),
            2
        );
        copy.apply(&frame);
        for k in [1u8, 2] {
            rec.begin_frame();
            draw_shapes(&mut rec, k);
            let mut w = Vec::new();
            rec.encode_frame(&mut w);
            let (f, _) = Frame::decode(&w).unwrap();
            // k 1: nothing new; k 2: both shapes moved.
            assert_eq!(f.added.len(), if k == 1 { 0 } else { 2 }, "k {k}");
            copy.apply(&f);
            let mut d = SoftCanvas::new(W, H);
            draw_shapes(&mut d, k);
            let mut p = SoftCanvas::new(W, H);
            replay(&f.ops, &copy, &mut p);
            assert_eq!(d.px, p.px, "k {k}");
            wire.extend_from_slice(&w);
        }
        assert_eq!(copy.bytes(), rec.store().bytes());
        // Junk in a frame with shapes is refused, never panicked on.
        for n in 0..wire.len().min(400) {
            let _ = Frame::decode(&wire[..n]);
        }
        for i in 0..wire.len().min(400) {
            let mut bad = wire.clone();
            bad[i] ^= 0xA5;
            if let Ok((f, _)) = Frame::decode(&bad) {
                let mut st = Store::new(usize::MAX);
                st.apply(&f);
                replay(&f.ops, &st, &mut SoftCanvas::new(W, H));
            }
        }
    }

    #[test]
    fn junk_is_refused_not_panicked_on() {
        let mut rec = Recorder::new(W, H);
        rec.begin_frame();
        draw(&mut rec, 3);
        let mut wire = Vec::new();
        rec.encode_frame(&mut wire);
        assert!(Frame::decode(&wire).is_ok());
        // Every cut short, and every byte changed, either reads or says why.
        for n in 0..wire.len() {
            assert!(Frame::decode(&wire[..n]).is_err(), "cut at {n}");
        }
        for i in 0..wire.len() {
            for flip in [0x01u8, 0x80, 0xFF] {
                let mut bad = wire.clone();
                bad[i] ^= flip;
                let _ = Frame::decode(&bad);
            }
        }
        // Counts and sizes far past the bytes are refused before allocating.
        let mut big = wire[..WIRE_HEADER].to_vec();
        big.extend_from_slice(&u32::MAX.to_le_bytes());
        let n = big.len() as u32;
        big[16..20].copy_from_slice(&n.to_le_bytes());
        assert!(Frame::decode(&big).is_err());
        assert!(Frame::decode(b"NOPE").is_err());
        assert!(Frame::decode_all(&[]).unwrap().is_empty());
    }
}
