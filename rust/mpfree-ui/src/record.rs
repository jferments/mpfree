//! A canvas that records a frame's drawing calls as a short list (a display
//! list), and plays the list back onto any other canvas.
//!
//! This is the seam between the screens and whatever puts their pixels on
//! the panel. Every widget draws through [`Canvas`]'s six calls; the
//! recorder keeps them as [`Op`]s, and [`Recorder::replay`] makes the same
//! calls on another canvas: [`crate::SoftCanvas`] today, a GPU renderer
//! next (the performance plan's "GPU drawing and compute"). Played back on
//! a `SoftCanvas` the picture is the same, bit for bit, as drawing straight
//! into it.
//!
//! Masks (text and strokes) and images are kept once each, by content, in
//! a [`Store`]: a frame's list names them by id, so a renderer in another
//! process receives each glyph once, not every frame. Entries no frame has
//! used for a while are dropped once the store passes its byte budget.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::canvas::{Canvas, Image};
use crate::color::Color;
use crate::geom::Rect;

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
    /// Mask and image calls, of those.
    pub masks: usize,
    pub images: usize,
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

impl Store {
    pub fn new(budget: usize) -> Store {
        Store {
            masks: HashMap::new(),
            images: HashMap::new(),
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

    /// Bytes held now.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.masks.len() + self.images.len()
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
            .filter(|(used, _)| used + KEEP_FRAMES <= frame)
            .collect();
        old.sort_unstable();
        for (_, id) in old {
            if self.bytes <= self.budget {
                break;
            }
            let (hash, bytes) = match (self.masks.remove(&id), self.images.remove(&id)) {
                (Some(e), _) => (e.hash, e.bytes),
                (_, Some(e)) => (e.hash, e.bytes),
                _ => continue,
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
}
