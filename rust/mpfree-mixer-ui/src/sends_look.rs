//! The Mix tab's Sends look (layout spec section 3a; the touch plan's A7,
//! Jay's second review 14.1): a row a strip, as on Effects, with the
//! strip's pan, its four sends and its output, each big enough to hit
//! (44 px). The Faders look's strips keep only a summary of them.
//!
//! - **Pan** drags sideways from where the finger lands (never jumps to
//!   it); the data wheel or SHIFT + its push as on a fader.
//! - **A send** is a **pre** toggle (lit while the send comes before the
//!   fader) and a level that drags as the pan does.
//! - **The output** opens the strip's output picker.
//!
//! The master's row has its pan and says where it goes.

use mpfree_fx::RETURNS;
use mpfree_ui::Rect;

/// The pan cell's width and the output's.
const PAN_W: i32 = 120;
const OUTPUT_W: i32 = 118;
/// Between cells.
const GAP: i32 = 8;
/// A send's pre toggle: square.
pub const PRE_W: i32 = 44;
/// A cell's height inside its row.
pub const CELL_H: i32 = 44;

/// A row's cells.
pub struct Row {
    pub name: Rect,
    pub pan: Rect,
    /// Each send's pre toggle and its level.
    pub pre: [Rect; RETURNS],
    pub send: [Rect; RETURNS],
    pub output: Rect,
}

/// The cells of a row at `row`, its name `name_w` wide (the Effects
/// look's).
pub fn row_parts(row: Rect, name_w: i32) -> Row {
    let name = row.take_left(name_w).0;
    let y = row.y + (row.h - CELL_H) / 2;
    let left = row.x + name_w + 8;
    let room = row.right() - left;
    let n = RETURNS as i32;
    let send_w = ((room - PAN_W - OUTPUT_W - (n + 1) * GAP) / n).max(PRE_W + 40);
    let pan = Rect::new(left, y, PAN_W, CELL_H);
    let mut x = pan.right() + GAP;
    let mut pre = [Rect::default(); RETURNS];
    let mut send = [Rect::default(); RETURNS];
    for k in 0..RETURNS {
        pre[k] = Rect::new(x, y, PRE_W, CELL_H);
        send[k] = Rect::new(x + PRE_W + GAP, y, send_w - PRE_W - GAP, CELL_H);
        x += send_w + GAP;
    }
    let output = Rect::new(x, y, (row.right() - x).max(0), CELL_H);
    Row {
        name,
        pan,
        pre,
        send,
        output,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rows_cells_are_44_px_and_fit_its_width() {
        let row = Rect::new(26, 208, 1088, 52);
        let r = row_parts(row, 176);
        assert_eq!(r.name.w, 176);
        for c in [r.pan, r.output].iter().chain(&r.pre).chain(&r.send) {
            assert_eq!(c.h, CELL_H);
            assert!(c.w >= CELL_H, "{c:?}");
            assert!(c.right() <= row.right());
        }
        for k in 1..RETURNS {
            assert!(r.pre[k].x >= r.send[k - 1].right() + GAP, "apart");
        }
        assert!(r.output.x >= r.send[RETURNS - 1].right() + GAP);
        assert!(
            r.send[0].w >= 100,
            "a level a finger can move: {}",
            r.send[0].w
        );
    }
}
