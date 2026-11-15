//! Anti-aliased strokes for curves and knobs. They are vector shapes
//! ([`mpfree_ui::Shape`], which these pages called a mask when it was one):
//! software rasterises them as it always did, and the GPU draws their parts
//! itself.

use mpfree_ui::{Canvas, Color, Rect};

/// Coverage for a rectangle of the screen, built from strokes, circles,
/// arcs and areas under curves; drawn in one colour.
pub use mpfree_ui::Shape as Mask;

/// Strokes `pts` in colour `c` inside `r`.
pub fn stroke(canvas: &mut dyn Canvas, r: Rect, pts: &[(f32, f32)], width: f32, c: Color) {
    let mut m = Mask::new(r);
    m.polyline(pts, width);
    m.draw(canvas, c);
}
