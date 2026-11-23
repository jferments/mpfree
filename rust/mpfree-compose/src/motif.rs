//! Motifs (phase 0C): a tune as places in the scale on a 1/16 grid,
//! where its phrases break, how alike two stretches are, and the classic
//! ways of changing a motif. The second prototype's, on mpfree's grid.
//!
//! Notes are scale places ([`Key::position`]), so moving a motif two
//! places is a sequence that stays in the key, and turning it upside
//! down mirrors it in the scale.

use mpfree_context::groove::STEP;
use mpfree_context::key::Key;
use mpfree_transport::time::Tick;

use crate::realize::PlainNote;

/// One note of a tune on the grid: its start and length in 1/16 steps
/// from the start of what was played, its place in the scale, how far it
/// sits above that place, and how loud.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GridNote {
    pub on: i32,
    pub len: i32,
    pub pos: i32,
    pub alt: i8,
    pub velocity: u8,
}

impl GridNote {
    pub fn pitch(&self, key: Key) -> u8 {
        key.pitch_at(self.pos, self.alt)
    }

    /// As a clip holds it, in ticks from the start of what was played.
    pub fn plain(&self, key: Key) -> PlainNote {
        PlainNote {
            at: Tick::from(self.on) * STEP,
            len: Tick::from(self.len.max(1)) * STEP,
            pitch: self.pitch(key),
            velocity: self.velocity,
        }
    }
}

/// Notes as one line on the grid: the highest note at each start.
pub fn line(notes: &[PlainNote], key: Key) -> Vec<GridNote> {
    let mut v: Vec<GridNote> = notes
        .iter()
        .map(|n| {
            let (pos, alt) = key.position(n.pitch);
            GridNote {
                on: (n.at as f64 / STEP as f64).round() as i32,
                len: ((n.len as f64 / STEP as f64).round() as i32).max(1),
                pos,
                alt,
                velocity: n.velocity,
            }
        })
        .collect();
    v.sort_by_key(|x| (x.on, std::cmp::Reverse(key.pitch_at(x.pos, x.alt))));
    v.dedup_by_key(|x| x.on);
    v
}

/// Bar `bar`'s notes (bars of `bar_steps`), counted from its start.
pub fn bar_of(line: &[GridNote], bar: i32, bar_steps: i32) -> Vec<GridNote> {
    let (a, b) = (bar * bar_steps, (bar + 1) * bar_steps);
    line.iter()
        .filter(|x| x.on >= a && x.on < b)
        .map(|x| GridNote { on: x.on - a, ..*x })
        .collect()
}

/// How strongly a phrase breaks between each note and the next, 0 to 1
/// (Cambouropoulos's Local Boundary Detection Model): from changes in the
/// leap, the time between starts and the rests, the time weighing most.
pub fn boundaries(line: &[GridNote]) -> Vec<f64> {
    let n = line.len();
    if n < 3 {
        return vec![0.0; n.saturating_sub(1)];
    }
    let profile = |xs: Vec<f64>| -> Vec<f64> {
        let m = xs.len();
        let change = |a: f64, b: f64| {
            if a + b == 0.0 {
                0.0
            } else {
                (a - b).abs() / (a + b)
            }
        };
        let mut s: Vec<f64> = (0..m)
            .map(|i| {
                let left = if i > 0 { change(xs[i - 1], xs[i]) } else { 0.0 };
                let right = if i + 1 < m {
                    change(xs[i], xs[i + 1])
                } else {
                    0.0
                };
                xs[i] * (left + right)
            })
            .collect();
        let top = s.iter().copied().fold(0.0, f64::max);
        if top > 0.0 {
            s.iter_mut().for_each(|x| *x /= top);
        }
        s
    };
    let leap = (0..n - 1)
        .map(|i| f64::from((line[i + 1].pos - line[i].pos).abs()))
        .collect();
    let gap = (0..n - 1)
        .map(|i| f64::from(line[i + 1].on - line[i].on))
        .collect();
    let rest = (0..n - 1)
        .map(|i| f64::from((line[i + 1].on - (line[i].on + line[i].len)).max(0)))
        .collect();
    let (p, o, r) = (profile(leap), profile(gap), profile(rest));
    (0..n - 1)
        .map(|i| 0.25 * p[i] + 0.5 * o[i] + 0.25 * r[i])
        .collect()
}

/// A line cut into motifs at its strongest breaks, three to eight notes
/// each.
pub fn motifs(line: &[GridNote]) -> Vec<Vec<GridNote>> {
    let b = boundaries(line);
    let mean = b.iter().sum::<f64>() / b.len().max(1) as f64;
    let mut cuts = vec![0usize];
    for i in 0..b.len() {
        let peak = (i == 0 || b[i] >= b[i - 1]) && (i + 1 >= b.len() || b[i] >= b[i + 1]);
        let since = i + 1 - cuts.last().copied().unwrap_or(0);
        if (peak && b[i] > mean.max(0.3) && since >= 3) || since >= 8 {
            cuts.push(i + 1);
        }
    }
    cuts.push(line.len());
    cuts.dedup();
    cuts.windows(2)
        .map(|w| line[w[0]..w[1]].to_vec())
        .filter(|m| !m.is_empty())
        .collect()
}

/// How alike two stretches of tune are, 0 to 1, whatever key they're
/// moved to, and its parts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Similarity {
    /// 0.35 leaps, 0.25 shape, 0.3 rhythm, 0.1 accents.
    pub total: f64,
    /// The leaps and lengths, as an edit distance.
    pub interval: f64,
    /// Which way the tune goes between every pair of notes.
    pub contour: f64,
    /// Starts in the same places.
    pub rhythm: f64,
    /// Loud notes in the same places.
    pub accent: f64,
}

impl Similarity {
    /// The tune's part alone (leaps and shape), which is what a bar's
    /// notes decide once its rhythm is set.
    pub fn tune(&self) -> f64 {
        (0.35 * self.interval + 0.25 * self.contour) / 0.6
    }
}

/// Leaps and lengths as an edit distance (after Mongeau and Sankoff).
fn interval_similarity(key: Key, a: &[GridNote], b: &[GridNote]) -> f64 {
    let steps = |m: &[GridNote]| -> Vec<(i32, i32)> {
        (1..m.len())
            .map(|i| {
                (
                    i32::from(m[i].pitch(key)) - i32::from(m[i - 1].pitch(key)),
                    m[i].len.max(1),
                )
            })
            .collect()
    };
    let (x, y) = (steps(a), steps(b));
    let (n, m) = (x.len(), y.len());
    if n == 0 && m == 0 {
        return 1.0;
    }
    let mut d = vec![vec![0.0f64; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i as f64;
    }
    for j in 0..=m {
        d[0][j] = j as f64;
    }
    for i in 1..=n {
        for j in 1..=m {
            let pitch = (f64::from((x[i - 1].0 - y[j - 1].0).abs()) / 5.0).min(1.0);
            let len = ((f64::from(x[i - 1].1) / f64::from(y[j - 1].1)).log2().abs() * 0.3).min(0.5);
            let swap = d[i - 1][j - 1] + (pitch + len).min(1.4);
            d[i][j] = swap.min(d[i - 1][j] + 1.0).min(d[i][j - 1] + 1.0);
        }
    }
    1.0 - (d[n][m] / n.max(m) as f64).min(1.0)
}

/// Shape (Marvin and Laprade's CSIM) on equal samples of both.
fn contour_similarity(key: Key, a: &[GridNote], b: &[GridNote]) -> f64 {
    let k = a.len().min(b.len());
    if k < 2 {
        return if a.len() == b.len() { 1.0 } else { 0.5 };
    }
    let sample = |m: &[GridNote]| -> Vec<i32> {
        (0..k)
            .map(|i| i32::from(m[i * (m.len() - 1) / (k - 1)].pitch(key)))
            .collect()
    };
    let (x, y) = (sample(a), sample(b));
    let (mut same, mut all) = (0u32, 0u32);
    for i in 0..k {
        for j in i + 1..k {
            all += 1;
            if (x[j] - x[i]).signum() == (y[j] - y[i]).signum() {
                same += 1;
            }
        }
    }
    f64::from(same) / f64::from(all)
}

/// Starts on the same steps; a step off counts half.
fn rhythm_similarity(a: &[GridNote], b: &[GridNote]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return if a.is_empty() && b.is_empty() {
            1.0
        } else {
            0.0
        };
    }
    let score = |x: &[GridNote], y: &[GridNote]| -> f64 {
        x.iter()
            .map(|n| {
                if y.iter().any(|m| m.on == n.on) {
                    1.0
                } else if y.iter().any(|m| (m.on - n.on).abs() == 1) {
                    0.5
                } else {
                    0.0
                }
            })
            .sum()
    };
    (score(a, b) + score(b, a)) / (a.len() + b.len()) as f64
}

/// Notes louder than the stretch's average, in the same places.
fn accent_similarity(a: &[GridNote], b: &[GridNote]) -> f64 {
    let accents = |m: &[GridNote]| -> Vec<i32> {
        let mean = m.iter().map(|x| f64::from(x.velocity)).sum::<f64>() / m.len().max(1) as f64;
        m.iter()
            .filter(|x| f64::from(x.velocity) > mean + 4.0)
            .map(|x| x.on)
            .collect()
    };
    let (x, y) = (accents(a), accents(b));
    if x.is_empty() && y.is_empty() {
        return 1.0;
    }
    let both = x.iter().filter(|o| y.contains(o)).count() as f64;
    2.0 * both / (x.len() + y.len()) as f64
}

/// How alike two motifs or bars are.
pub fn similarity(key: Key, a: &[GridNote], b: &[GridNote]) -> Similarity {
    let interval = interval_similarity(key, a, b);
    let contour = contour_similarity(key, a, b);
    let rhythm = rhythm_similarity(a, b);
    let accent = accent_similarity(a, b);
    Similarity {
        total: 0.35 * interval + 0.25 * contour + 0.3 * rhythm + 0.1 * accent,
        interval,
        contour,
        rhythm,
        accent,
    }
}

/// The classic changes. Every one keeps notes on the grid; moves are in
/// scale places, so a move stays in the key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transform {
    /// Up or down the scale.
    Move(i32),
    /// Upside down, around its first note.
    Invert,
    /// Its notes backwards, in the same rhythm.
    Backwards,
    /// Twice as slow.
    Augment,
    /// Twice as fast.
    Diminish,
    /// Its first notes.
    Head(usize),
    /// Its last notes.
    Tail(usize),
    /// Moved in time by some steps.
    Shift(i32),
    /// Its leaps made wider (above 1) or narrower.
    Widen(f64),
    /// Its last note on a scale place.
    EndOn(i32),
}

pub fn transform(m: &[GridNote], c: Transform) -> Vec<GridNote> {
    let Some(first) = m.first().copied() else {
        return Vec::new();
    };
    let mut v = m.to_vec();
    match c {
        Transform::Move(k) => v.iter_mut().for_each(|x| x.pos += k),
        Transform::Invert => v.iter_mut().for_each(|x| {
            x.pos = 2 * first.pos - x.pos;
            x.alt = 0;
        }),
        Transform::Backwards => {
            for (x, y) in v.iter_mut().zip(m.iter().rev()) {
                x.pos = y.pos;
                x.alt = y.alt;
            }
        }
        Transform::Augment => v.iter_mut().for_each(|x| {
            x.on = first.on + (x.on - first.on) * 2;
            x.len *= 2;
        }),
        Transform::Diminish => v.iter_mut().for_each(|x| {
            x.on = first.on + (x.on - first.on) / 2;
            x.len = (x.len / 2).max(1);
        }),
        Transform::Head(k) => v.truncate(k.max(1)),
        Transform::Tail(k) => {
            v.drain(..v.len().saturating_sub(k.max(1)));
        }
        Transform::Shift(s) => v.iter_mut().for_each(|x| x.on += s),
        Transform::Widen(f) => v.iter_mut().for_each(|x| {
            x.pos = first.pos + (f64::from(x.pos - first.pos) * f).round() as i32;
        }),
        Transform::EndOn(p) => {
            if let Some(l) = v.last_mut() {
                l.pos = p;
                l.alt = 0;
            }
        }
    }
    v
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use mpfree_context::key::Mode;

    pub(crate) fn g(on: i32, len: i32, pos: i32, velocity: u8) -> GridNote {
        GridNote {
            on,
            len,
            pos,
            alt: 0,
            velocity,
        }
    }

    /// The prototype's two-bar riff in C minor, as plain notes.
    pub(crate) fn riff() -> Vec<PlainNote> {
        [
            (0, 2, 60, 100),
            (2, 1, 63, 88),
            (3, 1, 67, 92),
            (4, 2, 65, 96),
            (6, 2, 63, 86),
            (8, 3, 62, 94),
            (11, 1, 63, 80),
            (12, 4, 60, 90),
            (16, 2, 67, 100),
            (18, 1, 68, 86),
            (19, 1, 67, 84),
            (20, 2, 65, 94),
            (22, 2, 63, 84),
            (24, 2, 62, 92),
            (26, 2, 63, 84),
            (28, 4, 65, 88),
        ]
        .iter()
        .map(|&(on, len, pitch, velocity)| PlainNote {
            at: on * STEP,
            len: len * STEP,
            pitch,
            velocity,
        })
        .collect()
    }

    const CM: Key = Key::new(0, Mode::Minor);

    #[test]
    fn a_line_is_the_top_note_at_each_start_and_back() {
        let mut notes = riff();
        // A lower note under the first: the line keeps the top one.
        notes.push(PlainNote {
            at: 0,
            len: 2 * STEP,
            pitch: 48,
            velocity: 90,
        });
        let l = line(&notes, CM);
        assert_eq!(l.len(), 16);
        assert_eq!(l[0].pitch(CM), 60);
        assert_eq!(l[1].on, 2);
        let back: Vec<PlainNote> = l.iter().map(|n| n.plain(CM)).collect();
        assert_eq!(back, riff());
        assert_eq!(bar_of(&l, 1, 16)[0], GridNote { on: 0, ..l[8] });
    }

    #[test]
    fn phrases_break_after_long_notes_and_rests() {
        let l = line(&riff(), CM);
        let b = boundaries(&l);
        assert_eq!(b.len(), 15);
        // The strongest break is after the long C that ends bar 1.
        let top = (0..b.len()).max_by(|&i, &j| b[i].total_cmp(&b[j])).unwrap();
        assert_eq!(top, 7);
        let ms = motifs(&l);
        assert!(ms.len() >= 2);
        assert!(ms.iter().all(|m| (1..=8).contains(&m.len())));
        assert_eq!(ms.iter().map(Vec::len).sum::<usize>(), 16);
    }

    #[test]
    fn a_moved_bar_is_alike_and_a_new_one_is_not() {
        let l = line(&riff(), CM);
        let bar = bar_of(&l, 0, 16);
        let same = similarity(CM, &bar, &bar);
        assert!((same.total - 1.0).abs() < 1e-9);
        // Moved a step up the scale: the same rhythm and shape, a few
        // leaps a semitone different.
        let moved = similarity(CM, &bar, &transform(&bar, Transform::Move(1)));
        assert_eq!(moved.rhythm, 1.0);
        assert_eq!(moved.contour, 1.0);
        assert!(moved.total > 0.9, "{}", moved.total);
        let upside = similarity(CM, &bar, &transform(&bar, Transform::Invert));
        assert!(upside.contour < 0.2, "{}", upside.contour);
        let other: Vec<GridNote> = (0..8).map(|i| g(i * 2 + 1, 1, 35 + (i % 3), 90)).collect();
        assert!(similarity(CM, &bar, &other).total < 0.5);
    }

    #[test]
    fn the_changes() {
        let m = vec![g(0, 2, 35, 90), g(2, 2, 37, 90), g(4, 4, 39, 90)];
        let pos = |v: &[GridNote]| v.iter().map(|x| x.pos).collect::<Vec<_>>();
        let on = |v: &[GridNote]| v.iter().map(|x| x.on).collect::<Vec<_>>();
        assert_eq!(pos(&transform(&m, Transform::Move(2))), vec![37, 39, 41]);
        assert_eq!(pos(&transform(&m, Transform::Invert)), vec![35, 33, 31]);
        assert_eq!(pos(&transform(&m, Transform::Backwards)), vec![39, 37, 35]);
        assert_eq!(on(&transform(&m, Transform::Augment)), vec![0, 4, 8]);
        assert_eq!(on(&transform(&m, Transform::Diminish)), vec![0, 1, 2]);
        assert_eq!(transform(&m, Transform::Head(2)).len(), 2);
        assert_eq!(pos(&transform(&m, Transform::Tail(1))), vec![39]);
        assert_eq!(on(&transform(&m, Transform::Shift(8))), vec![8, 10, 12]);
        assert_eq!(pos(&transform(&m, Transform::Widen(1.5))), vec![35, 38, 41]);
        assert_eq!(pos(&transform(&m, Transform::EndOn(35))), vec![35, 37, 35]);
        assert!(transform(&[], Transform::Invert).is_empty());
    }
}
