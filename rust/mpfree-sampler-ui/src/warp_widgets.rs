//! The Warp tab's picture: a warped region drawn in its beats, so a pinned
//! hit sits on its grid line.

use std::sync::Arc;

use std::cell::RefCell;

use mpfree_sampler::model::{BeatLine, Warp};
use mpfree_ui::wave::Peaks;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Touch, Widget};

/// A pin's beat in a [`EventKind::Marker`] from the picture, in these
/// parts of a beat.
pub const TICKS: f64 = 960.0;
/// Beats a bar, as the grid draws them.
pub const PER_BAR: f64 = 4.0;

/// The strip of pin handles and bar numbers along the top.
const RULER_H: i32 = 30;
/// The strip of hit ticks along the bottom.
const HITS_H: i32 = 12;
/// A touch that moves less than this is a tap.
const TAP_SLOP: i32 = 8;
/// How near a pin's line a touch takes hold of it.
const GRAB_PX: i32 = 18;
/// A pin's handle.
const HANDLE_W: i32 = 22;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    /// Dragging pin `index`, from `x0`.
    Pin { index: usize, x0: i32, moved: bool },
    /// Maybe a tap, until it moves; then a scroll from `first`.
    Wave { x0: i32, first: f64, moved: bool },
}

/// One layer's region in beats: the grid, the wave as it plays (stretched
/// between pins), the pins, the hits found in the sound and the playheads.
/// Touches come back as [`EventKind::Select`] (a pin's handle tapped),
/// [`EventKind::Marker`] (pin `index` dragged; `frame` is the beat under
/// the finger in [`TICKS`]), [`EventKind::Frame`] (a tap on the wave: the
/// source frame that plays there) and [`EventKind::View`] (scrolled:
/// `start` is the first beat shown, `frames_per_px` beats a pixel).
pub struct WarpView {
    pub title: String,
    pub right: String,
    pub peaks: Option<Arc<dyn Peaks>>,
    /// The source's rate, and the region in it.
    pub rate: u32,
    pub start: u64,
    pub end: u64,
    /// The region's warp: its tempo sets the beats, its pins bend them.
    pub warp: Warp,
    /// The pins move hits (the region stretches); otherwise they are drawn
    /// faint and the region plays evenly.
    pub pins_on: bool,
    /// Beats between grid lines.
    pub grid: f64,
    /// Hits found in the sound, source frames in order.
    pub hits: Vec<u64>,
    pub selected: Option<usize>,
    /// Source frames being played.
    pub playheads: Vec<u64>,
    /// Said over the picture when not empty.
    pub note: String,
    /// The beats shown: from `first`, `span` of them (`span` 0: the whole
    /// region).
    pub first: f64,
    pub span: f64,
    pub accent: Color,
    grab: Option<Grab>,
    painted: RefCell<Painted>,
}

/// What the last paint drew, so a playhead moving redraws only its strips.
#[derive(Default)]
struct Painted {
    /// A whole paint has happened and only the playheads changed since.
    only_heads: bool,
    xs: Vec<i32>,
}

impl WarpView {
    pub fn new(title: &str) -> WarpView {
        WarpView {
            title: title.to_string(),
            right: String::new(),
            peaks: None,
            rate: 44_100,
            start: 0,
            end: 1,
            warp: Warp::new(120.0),
            pins_on: false,
            grid: 0.25,
            hits: Vec::new(),
            selected: None,
            playheads: Vec::new(),
            note: String::new(),
            first: 0.0,
            span: 0.0,
            accent: Color::rgb(0xFFC145),
            grab: None,
            painted: RefCell::default(),
        }
    }

    /// Beats the region lasts.
    pub fn total(&self) -> f64 {
        self.warp
            .beats(self.end.saturating_sub(self.start), self.rate)
            .max(1e-6)
    }

    /// The beats shown: the first, and how many.
    pub fn shown(&self) -> (f64, f64) {
        if self.span > 0.0 {
            (self.first, self.span)
        } else {
            let t = self.total();
            (-t * 0.01, t * 1.02)
        }
    }

    /// The beat source frame `frame` plays on.
    pub fn beat_of(&self, frame: u64) -> f64 {
        self.line().beat_at(frame as f64)
    }

    /// The source frame that plays on `beat`.
    pub fn frame_of(&self, beat: f64) -> f64 {
        self.line().frame_at(beat)
    }

    /// The line the picture follows between frames and beats: through the
    /// pins only when they count.
    fn line(&self) -> BeatLine {
        if self.pins_on {
            self.warp.line(self.start, self.end, self.rate)
        } else {
            Warp {
                pins: Vec::new(),
                ..self.warp.clone()
            }
            .line(self.start, self.end, self.rate)
        }
    }

    /// Something besides the playheads changed: the next paint is whole.
    /// Call it after setting the other fields.
    pub fn changed(&mut self) {
        self.painted.get_mut().only_heads = false;
    }

    /// Where the voices are, in source frames. Only this changing redraws
    /// only the strips the playheads left and entered.
    pub fn set_playheads(&mut self, frames: &[u64]) {
        self.playheads.clear();
        self.playheads.extend_from_slice(frames);
    }

    /// Where each playhead is drawn across `wave`.
    fn head_xs(&self, line: &BeatLine, wave: Rect) -> Vec<i32> {
        self.playheads
            .iter()
            .map(|&f| self.x_of(line.beat_at(f as f64), wave))
            .filter(|x| *x >= wave.x && *x <= wave.right())
            .collect()
    }

    /// The ruler, the wave and the hits strip inside the panel `r`.
    pub fn areas(r: Rect) -> (Rect, Rect, Rect) {
        let body = r.inset(14).take_top(34).1;
        let (ruler, rest) = body.take_top(RULER_H);
        let (hits, wave) = rest.take_bottom(HITS_H);
        (ruler, wave, hits)
    }

    /// Where `beat` is across `plot`.
    pub fn x_of(&self, beat: f64, plot: Rect) -> i32 {
        let (first, span) = self.shown();
        plot.x + ((beat - first) / span * f64::from(plot.w)).round() as i32
    }

    /// The beat at `x` across `plot`.
    pub fn beat_at_x(&self, x: i32, plot: Rect) -> f64 {
        let (first, span) = self.shown();
        first + f64::from(x - plot.x) / f64::from(plot.w.max(1)) * span
    }

    /// The pin whose line is nearest `x`, within reach.
    fn pin_near(&self, x: i32, plot: Rect) -> Option<usize> {
        self.warp
            .pins
            .iter()
            .enumerate()
            .map(|(i, p)| (i, (self.x_of(p.beat, plot) - x).abs()))
            .filter(|(_, d)| *d <= GRAB_PX)
            .min_by_key(|(_, d)| *d)
            .map(|(i, _)| i)
    }
}

/// A beat as bar.beat.sixteenth, counted from 1.
pub fn beat_text(beat: f64) -> String {
    let sixteenths = (beat * 4.0).round().max(0.0) as i64;
    let rest = beat * 4.0 - sixteenths as f64;
    let (bar, in_bar) = (sixteenths / 16, sixteenths % 16);
    let mut s = format!("{}.{}.{}", bar + 1, in_bar / 4 + 1, in_bar % 4 + 1);
    // Off the sixteenth: by how many hundredths of one.
    let off = (rest * 100.0).round() as i64;
    if off != 0 {
        s.push_str(&format!(" {off:+}"));
    }
    s
}

impl Widget for WarpView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.right);
        let (ruler, wave, hits) = WarpView::areas(r);
        let small = t.size_small - 1.0;
        cx.canvas.fill_round_rect(wave, 8, t.bg);
        let total = self.total();
        let (first, span) = self.shown();
        let line = self.line();
        let clip = cx.canvas.clip();
        cx.canvas.set_clip(clip.intersect(Rect::new(
            wave.x,
            ruler.y,
            wave.w,
            hits.bottom() - ruler.y,
        )));
        // Past the region's end, shaded.
        let x_end = self.x_of(total, wave);
        if x_end < wave.right() {
            cx.canvas.fill_rect(
                Rect::new(x_end, wave.y, wave.right() - x_end, wave.h),
                t.surface.mix(t.bg, 0.5),
            );
        }
        // The grid: bars strong, beats lighter, the grid's steps faint.
        let px_per_beat = f64::from(wave.w) / span;
        let grid = if self.grid * px_per_beat >= 6.0 {
            self.grid
        } else {
            1.0
        };
        let k0 = (first.max(0.0) / grid).floor() as i64;
        let k1 = ((first + span).min(total) / grid).ceil() as i64;
        let bar_px = px_per_beat * PER_BAR;
        let label_every = (48.0 / bar_px).ceil().max(1.0) as i64;
        for k in k0..=k1 {
            let b = k as f64 * grid;
            if b < 0.0 || b > total + 1e-9 {
                continue;
            }
            let x = self.x_of(b, wave);
            let bar = (b / PER_BAR).fract().abs() < 1e-9;
            let beat = b.fract().abs() < 1e-9;
            let color = if bar {
                t.bg.mix(t.text_dim, 0.55)
            } else if beat {
                t.bg.mix(t.text_faint, 0.45)
            } else {
                t.bg.mix(t.text_faint, 0.2)
            };
            cx.canvas.fill_rect(Rect::new(x, wave.y, 1, wave.h), color);
            if bar {
                let n = (b / PER_BAR).round() as i64;
                cx.canvas
                    .fill_rect(Rect::new(x, ruler.bottom() - 8, 1, 8), t.text_dim);
                if n % label_every == 0 {
                    let lr = Rect::new(x + 4, ruler.y, 40, ruler.h - 6);
                    cx.text(
                        FontId::Mono,
                        small,
                        lr,
                        Align::Left,
                        t.text_dim,
                        &(n + 1).to_string(),
                    );
                }
            } else if beat {
                cx.canvas
                    .fill_rect(Rect::new(x, ruler.bottom() - 4, 1, 4), t.text_faint);
            }
        }
        // The wave as it plays: each column's beats back to source frames.
        if let Some(peaks) = &self.peaks {
            let ch = peaks.channels().clamp(1, 2);
            let lane_h = (wave.h - 4) / ch as i32;
            let frame = |b: f64| line.frame_at(b);
            let color = t.text_dim;
            let x0 = wave.x.max(self.x_of(0.0, wave));
            let x1 = wave.right().min(x_end);
            let mut f_prev = frame(self.beat_at_x(x0, wave));
            for x in x0..x1 {
                let f_next = frame(self.beat_at_x(x + 1, wave));
                let (a, b) = (
                    f_prev.clamp(self.start as f64, self.end as f64),
                    f_next.clamp(self.start as f64, self.end as f64),
                );
                f_prev = f_next;
                if b <= a {
                    continue;
                }
                for c in 0..ch {
                    let col = peaks.column(c, a, b);
                    let mid = wave.y + 2 + c as i32 * lane_h + lane_h / 2;
                    let half = (lane_h / 2 - 2).max(1) as f32;
                    let top = mid - (col.max.clamp(-1.0, 1.0) * half) as i32;
                    let bot = mid - (col.min.clamp(-1.0, 1.0) * half) as i32;
                    cx.canvas
                        .fill_rect(Rect::new(x, top, 1, (bot - top).max(1)), color);
                }
            }
        }
        // The hits found, as ticks along the bottom; the pinned ones lit.
        cx.canvas.fill_rect(hits, t.surface);
        for &h in &self.hits {
            if h <= self.start || h >= self.end {
                continue;
            }
            let x = self.x_of(line.beat_at(h as f64), wave);
            if x < wave.x || x > wave.right() {
                continue;
            }
            let pinned = self.warp.pins.iter().any(|p| p.frame == h);
            let color = if pinned && self.pins_on {
                self.accent
            } else {
                t.text_faint
            };
            cx.canvas
                .fill_rect(Rect::new(x - 1, hits.y + 2, 2, hits.h - 2), color);
        }
        // The pins: a line through the wave and a numbered handle.
        for (i, p) in self.warp.pins.iter().enumerate() {
            let x = self.x_of(p.beat, wave);
            let on = self.selected == Some(i);
            let color = if !self.pins_on {
                t.text_faint
            } else if on {
                t.text
            } else {
                self.accent
            };
            cx.canvas.fill_rect(
                Rect::new(x - i32::from(on), wave.y, 1 + 2 * i32::from(on), wave.h),
                color,
            );
            let hr = Rect::new(x - HANDLE_W / 2, ruler.y + 2, HANDLE_W, ruler.h - 6);
            cx.canvas.fill_round_rect(hr, 6, color);
            let label = (i + 1).to_string();
            cx.text(FontId::Bold, small, hr, Align::Center, t.bg, &label);
        }
        let xs = self.head_xs(&line, wave);
        for &x in &xs {
            cx.canvas.fill_rect(Rect::new(x, wave.y, 2, wave.h), t.good);
        }
        cx.canvas.set_clip(clip);
        {
            let mut painted = self.painted.borrow_mut();
            painted.only_heads = true;
            painted.xs = xs;
        }
        if !self.note.is_empty() {
            let nr = Rect::new(wave.x + 20, wave.y + wave.h / 2 - 16, wave.w - 40, 32);
            cx.canvas.fill_round_rect(nr.inset(-6), 8, t.surface);
            cx.text(
                FontId::Bold,
                t.size_body,
                nr,
                Align::Center,
                t.text,
                &self.note,
            );
        }
    }

    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let painted = self.painted.borrow();
        if !painted.only_heads {
            return None;
        }
        let (_, wave, _) = WarpView::areas(r);
        let mut xs = painted.xs.clone();
        xs.extend(self.head_xs(&self.line(), wave));
        // The note over the middle is drawn again whole if a strip
        // crosses it.
        if !self.note.is_empty() && !xs.is_empty() {
            return None;
        }
        Some(
            xs.into_iter()
                .map(|x| Rect::new(x - 1, wave.y, 4, wave.h))
                .collect(),
        )
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let (ruler, wave, _) = WarpView::areas(r);
        match phase {
            Touch::Down => {
                let near = if y < wave.y || self.pins_on {
                    self.pin_near(x, wave)
                } else {
                    None
                };
                // In the ruler only a handle takes hold; in the wave, a line.
                self.grab = Some(match near {
                    Some(index) if y >= ruler.y => Grab::Pin {
                        index,
                        x0: x,
                        moved: false,
                    },
                    _ => Grab::Wave {
                        x0: x,
                        first: self.shown().0,
                        moved: false,
                    },
                });
                None
            }
            Touch::Move => match self.grab.as_mut()? {
                Grab::Pin { index, x0, moved } => {
                    if !*moved && (x - *x0).abs() < TAP_SLOP {
                        return None;
                    }
                    *moved = true;
                    let index = *index;
                    let b = self.beat_at_x(x, wave).max(0.0);
                    Some(EventKind::Marker {
                        index,
                        frame: (b * TICKS).round() as u64,
                    })
                }
                Grab::Wave { x0, first, moved } => {
                    if !*moved && (x - *x0).abs() < TAP_SLOP {
                        return None;
                    }
                    *moved = true;
                    let (x0, first) = (*x0, *first);
                    if self.span <= 0.0 {
                        // The whole region is in view: nothing to scroll.
                        return None;
                    }
                    let per_px = self.span / f64::from(wave.w.max(1));
                    let max = (self.total() - self.span * 0.5).max(0.0);
                    self.first = (first - f64::from(x - x0) * per_px).clamp(-self.span * 0.5, max);
                    self.changed();
                    Some(EventKind::View {
                        start: self.first,
                        frames_per_px: per_px,
                    })
                }
            },
            Touch::Up => match self.grab.take()? {
                Grab::Pin {
                    index,
                    moved: false,
                    ..
                } => Some(EventKind::Select(index)),
                Grab::Wave { moved: false, .. } if y >= wave.y => {
                    let f = self.frame_of(self.beat_at_x(x, wave));
                    Some(EventKind::Frame(f.round().max(0.0) as u64))
                }
                _ => None,
            },
            Touch::Cancel => {
                self.grab = None;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beats_read_as_bar_beat_sixteenth() {
        assert_eq!(beat_text(0.0), "1.1.1");
        assert_eq!(beat_text(1.0), "1.2.1");
        assert_eq!(beat_text(4.75), "2.1.4");
        assert_eq!(beat_text(4.8), "2.1.4 +20");
        assert_eq!(beat_text(5.2), "2.2.2 −20".replace('−', "-"));
    }
}
