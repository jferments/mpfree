//! The EQ's page in the chain editor: a curve to shape by hand (Jay, 15 Nov
//! 2026: "Approve as drawn" on the parametric EQ mockups, boards B and C).
//!
//! - **The graph** (the top of the page): 20 Hz to 20 kHz across, ±18 dB
//!   up, the whole response in white over each band's own shape in its
//!   colour (the picked band's stronger), and a dot per band in use: its
//!   number (LC and HC for the cuts). A finger on a dot picks that band;
//!   once it has moved [`LIFT`] pixels the dot follows it, frequency across
//!   and gain up (a cut or a notch only across), with a readout over it;
//!   a second finger spreads or pinches its Q. Two taps in one place on
//!   the empty graph add a bell there (the first Unused band).
//! - **The band panel** (under the graph): a chip per band in use, + Band,
//!   then the picked band's type (Low cut, Low shelf, Bell, Notch, High
//!   shelf, High cut: picking a cut for a band moves it to that cut, and
//!   back), its Freq, Gain (a cut: its Slope, a tap steps it) and Q, On
//!   (Bypassed keeps it but isn't heard) and Remove (Unused; a cut: off).
//!
//! The picked band is the one whose control was last touched (the chain
//! editor's focus), so the Modulate row and the Q-Links follow it: the
//! Q-Links turn its Freq, Gain and Q, and the EQ's Output
//! ([`crate::state::eq_four`]). What is drawn comes from
//! [`mpfree_fx::Eq::response_db`] fed the page's values, so it is what
//! plays. The page keeps a copy of the values and moves it under the
//! finger at once; what the touches mean goes out as [`ChainAction`]s.

use mpfree_fx::ids::eq as ids;
use mpfree_fx::mpfree_node::Node;
use mpfree_param::ParamAddr;
use mpfree_synth_ui::draw::Mask;
use mpfree_ui::{Align, Color, FontId, PaintCx, Rect, Target};

use crate::state::ParamView;
use crate::view::ChainAction;

/// How many parameters the EQ has.
const N: usize = ids::LISTEN as usize + 1;
/// The graph's axes.
const F_LO: f32 = 20.0;
const F_HI: f32 = 20_000.0;
const DB: f32 = 18.0;
/// Points across the curve.
const K: usize = 160;
/// Pixels a finger moves on a dot before the dot follows it (spec 4h:
/// less is a tap).
pub const LIFT: i32 = 12;
/// How far from a dot's middle a finger still takes it.
const HIT: f32 = 26.0;
/// The graph's share of the page; the band panel has the rest.
const GRAPH_SHARE: f32 = 0.63;
/// The band panel's chip row, the type row and the value row.
const CHIP_W: i32 = 74;
const CHIP_H: i32 = 46;
const TYPE_W: i32 = 98;
const TYPE_H: i32 = 40;
const FIELD_W: i32 = 150;
const ROW_H: i32 = 44;

/// A band's colour (bands 1 to 4 wear the Q-Links' colours).
pub const BAND_COLORS: [Color; 8] = [
    Color::rgb(0x6EA8FE),
    Color::rgb(0x3DDC97),
    Color::rgb(0xFFC145),
    Color::rgb(0xD46CFF),
    Color::rgb(0xFF8A65),
    Color::rgb(0x4FC3C9),
    Color::rgb(0xF06292),
    Color::rgb(0xAED581),
];
/// The cuts' colour.
const CUT_COLOR: Color = Color::rgb(0xB8C2D6);

/// A dot on the graph: one of the eight bands, or a cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Band(usize),
    Low,
    High,
}

impl Band {
    /// "3", "LC", "HC".
    pub fn label(self) -> String {
        match self {
            Band::Band(b) => (b + 1).to_string(),
            Band::Low => "LC".into(),
            Band::High => "HC".into(),
        }
    }
    /// "Band 3", "Low cut".
    pub fn name(self) -> String {
        match self {
            Band::Band(b) => format!("Band {}", b + 1),
            Band::Low => "Low cut".into(),
            Band::High => "High cut".into(),
        }
    }
    pub fn color(self) -> Color {
        match self {
            Band::Band(b) => BAND_COLORS[b % BAND_COLORS.len()],
            _ => CUT_COLOR,
        }
    }
    /// The band a parameter id belongs to.
    pub fn of(id: u16) -> Option<Band> {
        match id {
            ids::LOW_CUT | ids::LOW_SLOPE => Some(Band::Low),
            ids::HIGH_CUT | ids::HIGH_SLOPE => Some(Band::High),
            _ => ids::band_of(id).map(|(b, _)| Band::Band(b)),
        }
    }
    /// The parameter the band's dot moves up and down, else across: what
    /// picking it focuses.
    pub fn main_id(self) -> u16 {
        match self {
            Band::Band(b) => ids::gain(b),
            Band::Low => ids::LOW_CUT,
            Band::High => ids::HIGH_CUT,
        }
    }
}

/// The type chips, in the panel's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    LowCut,
    LowShelf,
    Bell,
    Notch,
    HighShelf,
    HighCut,
}

pub const KINDS: [Kind; 6] = [
    Kind::LowCut,
    Kind::LowShelf,
    Kind::Bell,
    Kind::Notch,
    Kind::HighShelf,
    Kind::HighCut,
];

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::LowCut => "Low cut",
            Kind::LowShelf => "Low shelf",
            Kind::Bell => "Bell",
            Kind::Notch => "Notch",
            Kind::HighShelf => "High shelf",
            Kind::HighCut => "High cut",
        }
    }
    /// The band's Type value for a band kind.
    fn type_value(self) -> Option<f32> {
        match self {
            Kind::Bell => Some(0.0),
            Kind::LowShelf => Some(1.0),
            Kind::HighShelf => Some(2.0),
            Kind::Notch => Some(3.0),
            _ => None,
        }
    }
    fn of_type(v: f32) -> Kind {
        match v as usize {
            1 => Kind::LowShelf,
            2 => Kind::HighShelf,
            3 => Kind::Notch,
            _ => Kind::Bell,
        }
    }
}

/// What the band panel's buttons are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// A band's chip: pick it.
    Chip(Band),
    /// + Band: a bell at 1 kHz in the first Unused band.
    Add,
    Kind(Kind),
    /// The second value: a cut's slope (a tap steps 12 / 24 dB).
    Slope,
    On,
    /// Hear the picked band alone (the EQ's Listen); a tap again stops.
    Listen,
    Remove,
}

/// Where the page's parts go in its rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct EqLayout {
    /// The graph's card, and the plot inside it (the axes' labels round
    /// it).
    pub graph: Rect,
    pub plot: Rect,
    /// The band panel's card, its title line, and the three value fields
    /// (Freq, Gain or Slope, Q).
    pub panel: Rect,
    pub title: Rect,
    pub fields: [Rect; 3],
    pub buttons: Vec<(Rect, Button)>,
}

/// The finger on the page.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    /// On a dot: where it went down, and whether it has lifted.
    Dot {
        band: Band,
        x0: i32,
        y0: i32,
        moved: bool,
    },
    /// On the empty graph (perhaps the first of two taps).
    Graph { x: i32, y: i32 },
    /// Elsewhere on the page (a button took it).
    Other,
}

/// The second finger while a dot is held: its Q follows the spread.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pinch {
    finger: u64,
    x: i32,
    y: i32,
    d0: f32,
    q0: f32,
}

/// See the module docs.
#[derive(Clone, Debug, Default)]
pub struct EqPage {
    /// Every parameter's value and address, by id (empty until a state
    /// with the EQ comes).
    vals: Vec<f32>,
    addrs: Vec<Option<ParamAddr>>,
    /// The band last touched (from the editor's focus).
    focus: Option<Band>,
    /// The curve, and each band's own shape, in dB at [`K`] points.
    curve: Vec<f32>,
    shapes: Vec<(Band, Vec<f32>)>,
    grab: Option<(u64, Grab)>,
    pinch: Option<Pinch>,
    /// The last tap on the empty graph, for a second one in the same place.
    tapped: Option<(i32, i32)>,
    /// Parameters set while a finger is down (Release goes out on the lift).
    turned: Vec<ParamAddr>,
}

/// The frequency at `x` (0 to 1) across the axis, and back.
fn x_freq(x: f32) -> f32 {
    F_LO * (F_HI / F_LO).powf(x.clamp(0.0, 1.0))
}
fn freq_x(hz: f32) -> f32 {
    ((hz.max(1.0) / F_LO).ln() / (F_HI / F_LO).ln()).clamp(0.0, 1.0)
}

/// Where everything goes in the page's rectangle `r`.
pub fn layout(r: Rect, page: &EqPage) -> EqLayout {
    let gh = (r.h as f32 * GRAPH_SHARE) as i32;
    let graph = Rect::new(r.x, r.y, r.w, gh);
    let plot = Rect::new(graph.x + 44, graph.y + 12, graph.w - 56, graph.h - 44);
    let panel = Rect::new(
        r.x,
        graph.bottom() + 10,
        r.w,
        r.bottom() - graph.bottom() - 10,
    );
    let inner = panel.inset_xy(12, 10);
    let mut buttons = Vec::new();
    let mut x = inner.x;
    for b in page.used() {
        buttons.push((Rect::new(x, inner.y, CHIP_W, CHIP_H), Button::Chip(b)));
        x += CHIP_W + 6;
    }
    if page.free().is_some() && x + CHIP_W <= inner.right() {
        buttons.push((Rect::new(x, inner.y, CHIP_W, CHIP_H), Button::Add));
    }
    let title = Rect::new(inner.x, inner.y + CHIP_H + 10, inner.w, 22);
    let rest = Rect::new(
        inner.x,
        title.bottom() + 2,
        inner.w,
        inner.bottom() - title.bottom() - 2,
    );
    let gap = ((rest.h - TYPE_H - ROW_H) / 3).max(4);
    let ty = rest.y + gap;
    let vy = ty + TYPE_H + gap;
    for (k, kind) in KINDS.iter().enumerate() {
        let b = Rect::new(rest.x + k as i32 * (TYPE_W + 6), ty, TYPE_W, TYPE_H);
        buttons.push((b, Button::Kind(*kind)));
    }
    let fields = [0, 1, 2].map(|k| Rect::new(rest.x + k * (FIELD_W + 8), vy, FIELD_W, ROW_H));
    if page.picked().is_some_and(|b| page.is_cut(b)) {
        buttons.push((fields[1], Button::Slope));
    }
    let bx = fields[2].right() + 18;
    buttons.push((Rect::new(bx, vy, 76, ROW_H), Button::On));
    buttons.push((Rect::new(bx + 82, vy, 88, ROW_H), Button::Listen));
    buttons.push((Rect::new(bx + 176, vy, 96, ROW_H), Button::Remove));
    EqLayout {
        graph,
        plot,
        panel,
        title,
        fields,
        buttons,
    }
}

impl EqPage {
    /// Reads the EQ's values from the page's parameters (`params` hold
    /// every one of the EQ's, [`crate::state::eq_page`]) and the control
    /// last touched.
    pub fn update(&mut self, params: &[ParamView], focus: Option<ParamAddr>) {
        self.vals = vec![0.0; N];
        self.addrs = vec![None; N];
        for p in params {
            let id = p.info.id as usize;
            if id < N {
                self.vals[id] = p.value;
                self.addrs[id] = Some(p.addr);
            }
        }
        self.focus = focus
            .filter(|f| self.addrs.contains(&Some(*f)))
            .and_then(|f| Band::of(f.param));
        self.redraw();
    }

    fn v(&self, id: u16) -> f32 {
        self.vals.get(id as usize).copied().unwrap_or(0.0)
    }

    /// True once a state with the EQ has come.
    pub fn loaded(&self) -> bool {
        !self.vals.is_empty()
    }

    fn is_cut(&self, b: Band) -> bool {
        matches!(b, Band::Low | Band::High)
    }

    /// The bands with a dot: the low cut while it is on, every band not
    /// Unused, the high cut while it is on.
    pub fn used(&self) -> Vec<Band> {
        let mut out = Vec::new();
        if self.v(ids::LOW_SLOPE) > 0.0 {
            out.push(Band::Low);
        }
        for b in 0..ids::BANDS {
            if self.v(ids::band_use(b)) != ids::USE_UNUSED {
                out.push(Band::Band(b));
            }
        }
        if self.v(ids::HIGH_SLOPE) > 0.0 {
            out.push(Band::High);
        }
        out
    }

    /// The first Unused band, for + Band.
    pub fn free(&self) -> Option<usize> {
        (0..ids::BANDS).find(|&b| self.v(ids::band_use(b)) == ids::USE_UNUSED)
    }

    /// The band the panel shows: the one last touched while it has a dot,
    /// else the first with one.
    pub fn picked(&self) -> Option<Band> {
        let used = self.used();
        self.focus
            .filter(|b| used.contains(b))
            .or_else(|| used.first().copied())
    }

    /// The type chip lit for band `b`.
    pub fn kind(&self, b: Band) -> Kind {
        match b {
            Band::Low => Kind::LowCut,
            Band::High => Kind::HighCut,
            Band::Band(n) => Kind::of_type(self.v(ids::band_type(n))),
        }
    }

    /// An EQ at these values with only `only` (None: everything) heard.
    fn scratch(&self, only: Option<Band>) -> mpfree_fx::Eq {
        let mut e = mpfree_fx::Eq::new();
        for (id, &v) in self.vals.iter().enumerate() {
            e.set_param(id as u16, v);
        }
        if let Some(keep) = only {
            if keep != Band::Low {
                e.set_param(ids::LOW_SLOPE, 0.0);
            }
            if keep != Band::High {
                e.set_param(ids::HIGH_SLOPE, 0.0);
            }
            for b in 0..ids::BANDS {
                if keep != Band::Band(b) {
                    e.set_param(ids::band_use(b), ids::USE_UNUSED);
                }
            }
            e.set_param(ids::OUTPUT, 0.0);
        }
        e
    }

    /// Works out the curve and the shapes again from the values.
    fn redraw(&mut self) {
        if !self.loaded() {
            return;
        }
        let hz: Vec<f32> = (0..K).map(|k| x_freq(k as f32 / (K - 1) as f32)).collect();
        let mut db = vec![0.0; K];
        self.scratch(None).response_db(&hz, &mut db);
        self.curve = db;
        self.shapes = self
            .used()
            .into_iter()
            .map(|b| {
                let mut d = vec![0.0; K];
                if self.on(b) {
                    self.scratch(Some(b)).response_db(&hz, &mut d);
                }
                (b, d)
            })
            .collect();
    }

    /// Band `b` is heard (a cut with a dot always is).
    pub fn on(&self, b: Band) -> bool {
        match b {
            Band::Band(n) => self.v(ids::band_use(n)) == ids::USE_ON,
            _ => true,
        }
    }

    /// The curve in dB at frequency `hz` (the nearest drawn point).
    pub fn curve_db(&self, hz: f32) -> f32 {
        if self.curve.is_empty() {
            return 0.0;
        }
        let k = (freq_x(hz) * (K - 1) as f32).round() as usize;
        self.curve[k.min(K - 1)]
    }

    /// Band `b`'s dot, as a share of the plot (x across, y up).
    pub fn dot(&self, b: Band) -> (f32, f32) {
        let up = |db: f32| (0.5 + db / (2.0 * DB)).clamp(0.04, 0.96);
        match b {
            Band::Band(n) => {
                let x = freq_x(self.v(ids::freq(n)));
                let flat = self.kind(b) == Kind::Notch;
                (x, if flat { 0.5 } else { up(self.v(ids::gain(n))) })
            }
            Band::Low => {
                let f = self.v(ids::LOW_CUT);
                (freq_x(f), up(self.curve_db(f)))
            }
            Band::High => {
                let f = self.v(ids::HIGH_CUT);
                (freq_x(f), up(self.curve_db(f)))
            }
        }
    }

    fn to_screen(plot: Rect, (x, y): (f32, f32)) -> (f32, f32) {
        (
            plot.x as f32 + x * plot.w as f32,
            plot.bottom() as f32 - y * plot.h as f32,
        )
    }

    /// The dot nearest `(x, y)` within reach of a finger.
    fn dot_at(&self, plot: Rect, x: i32, y: i32) -> Option<Band> {
        self.used()
            .into_iter()
            .map(|b| {
                let (dx, dy) = Self::to_screen(plot, self.dot(b));
                (b, (dx - x as f32).hypot(dy - y as f32))
            })
            .filter(|&(_, d)| d <= HIT)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(b, _)| b)
    }

    /// Sets parameter `id` here at once and tells the app.
    fn set(&mut self, id: u16, value: f32, out: &mut Vec<ChainAction>) {
        let Some(addr) = self.addrs.get(id as usize).copied().flatten() else {
            return;
        };
        if self.v(id) == value {
            return;
        }
        self.vals[id as usize] = value;
        out.push(ChainAction::Set { addr, value });
        if !self.turned.contains(&addr) {
            self.turned.push(addr);
        }
    }

    /// Sends Release for everything set since the last one.
    fn release(&mut self, out: &mut Vec<ChainAction>) {
        out.extend(self.turned.drain(..).map(ChainAction::Release));
    }

    /// The band Listen plays, if it is on.
    pub fn listening(&self) -> Option<usize> {
        match self.v(ids::LISTEN) as usize {
            0 => None,
            n => Some(n - 1),
        }
    }

    /// Turns Listen off (the page is closing). Returns the actions.
    pub fn stop_listening(&mut self) -> Vec<ChainAction> {
        let mut out = Vec::new();
        if self.listening().is_some() {
            self.set(ids::LISTEN, 0.0, &mut out);
            self.release(&mut out);
        }
        out
    }

    /// Picks band `b`: the editor focuses its main control. Listen follows
    /// the pick (a cut has nothing to listen to: it stops).
    fn pick(&mut self, b: Band, out: &mut Vec<ChainAction>) {
        if self.listening().is_some() {
            let to = match b {
                Band::Band(n) => n as f32 + 1.0,
                _ => 0.0,
            };
            self.set(ids::LISTEN, to, out);
        }
        if self.focus != Some(b) {
            self.focus = Some(b);
            if let Some(addr) = self.addrs.get(b.main_id() as usize).copied().flatten() {
                out.push(ChainAction::Focus(addr));
            }
        }
    }

    /// Puts a bell at `hz` and `db` in the first Unused band and picks it.
    fn add(&mut self, hz: f32, db: f32, out: &mut Vec<ChainAction>) -> bool {
        let Some(b) = self.free() else {
            return false;
        };
        self.set(ids::band_type(b), 0.0, out);
        self.set(ids::freq(b), hz.round().clamp(F_LO, F_HI), out);
        self.set(ids::gain(b), (db * 2.0).round() / 2.0, out);
        self.set(ids::q(b), 1.0, out);
        self.set(ids::band_use(b), ids::USE_ON, out);
        self.release(out);
        self.pick(Band::Band(b), out);
        self.redraw();
        true
    }

    /// Moves band `b`'s dot to `(fx, fy)`, shares of the plot.
    fn drag_to(&mut self, b: Band, fx: f32, fy: f32, out: &mut Vec<ChainAction>) {
        let hz = x_freq(fx).round();
        match b {
            Band::Low => self.set(ids::LOW_CUT, hz.clamp(20.0, 2_000.0), out),
            Band::High => self.set(ids::HIGH_CUT, hz.clamp(1_000.0, 20_000.0), out),
            Band::Band(n) => {
                self.set(ids::freq(n), hz, out);
                if self.kind(b) != Kind::Notch {
                    let db = ((fy - 0.5) * 2.0 * DB).clamp(-DB, DB);
                    self.set(ids::gain(n), (db * 2.0).round() / 2.0, out);
                }
            }
        }
        self.redraw();
    }

    /// Multiplies the picked band's Q by `by` (a cut: up steps it to 24
    /// dB, down to 12).
    pub fn scale_q(&mut self, b: Band, by: f32, out: &mut Vec<ChainAction>) {
        match b {
            Band::Low | Band::High => {
                let id = if b == Band::Low {
                    ids::LOW_SLOPE
                } else {
                    ids::HIGH_SLOPE
                };
                self.set(id, if by > 1.0 { 2.0 } else { 1.0 }, out);
            }
            Band::Band(n) => {
                let q = (self.v(ids::q(n)) * by).clamp(0.1, 18.0);
                self.set(ids::q(n), (q * 100.0).round() / 100.0, out);
            }
        }
        self.redraw();
    }

    /// The data wheel on band `b`'s dot: its Q, 6 % a detent.
    pub fn wheel(&mut self, b: Band, delta: i32, out: &mut Vec<ChainAction>) {
        self.pick(b, out);
        self.scale_q(b, 1.06f32.powi(delta), out);
        self.release(out);
    }

    /// A finger went down at `(x, y)` in the page `r`. Returns true when
    /// the page took it.
    pub fn down(&mut self, finger: u64, x: i32, y: i32, r: Rect, out: &mut Vec<ChainAction>) {
        let l = layout(r, self);
        if let Some((_, Grab::Dot { band, .. })) = self.grab {
            // A second finger while a dot is held: pinch its Q.
            if let Band::Band(n) = band {
                let (dx, dy) = Self::to_screen(l.plot, self.dot(band));
                self.pinch = Some(Pinch {
                    finger,
                    x,
                    y,
                    d0: (dx - x as f32).hypot(dy - y as f32).max(20.0),
                    q0: self.v(ids::q(n)),
                });
            }
            return;
        }
        if l.graph.contains(x, y) {
            if let Some(b) = self.dot_at(l.plot, x, y) {
                self.tapped = None;
                self.pick(b, out);
                self.grab = Some((
                    finger,
                    Grab::Dot {
                        band: b,
                        x0: x,
                        y0: y,
                        moved: false,
                    },
                ));
                return;
            }
            self.grab = Some((finger, Grab::Graph { x, y }));
            return;
        }
        self.tapped = None;
        self.grab = Some((finger, Grab::Other));
        let Some(&(_, button)) = l.buttons.iter().find(|(b, _)| b.contains(x, y)) else {
            return;
        };
        self.press(button, out);
    }

    /// A button of the band panel (a tap, or the wheel's push on it).
    pub fn press(&mut self, button: Button, out: &mut Vec<ChainAction>) {
        let picked = self.picked();
        match (button, picked) {
            (Button::Chip(b), _) => self.pick(b, out),
            (Button::Add, _) => {
                self.add(1_000.0, 0.0, out);
            }
            (Button::Kind(k), Some(b)) => self.retype(b, k, out),
            (Button::Slope, Some(b @ (Band::Low | Band::High))) => {
                let id = if b == Band::Low {
                    ids::LOW_SLOPE
                } else {
                    ids::HIGH_SLOPE
                };
                let to = if self.v(id) >= 2.0 { 1.0 } else { 2.0 };
                self.set(id, to, out);
            }
            (Button::On, Some(Band::Band(n))) => {
                let to = if self.v(ids::band_use(n)) == ids::USE_ON {
                    ids::USE_BYPASSED
                } else {
                    ids::USE_ON
                };
                self.set(ids::band_use(n), to, out);
            }
            (Button::Listen, Some(Band::Band(n))) => {
                let to = if self.listening() == Some(n) {
                    0.0
                } else {
                    n as f32 + 1.0
                };
                self.set(ids::LISTEN, to, out);
            }
            (Button::Remove, Some(b)) => match b {
                Band::Low => self.set(ids::LOW_SLOPE, 0.0, out),
                Band::High => self.set(ids::HIGH_SLOPE, 0.0, out),
                Band::Band(n) => {
                    if self.listening() == Some(n) {
                        self.set(ids::LISTEN, 0.0, out);
                    }
                    self.set(ids::band_use(n), ids::USE_UNUSED, out);
                    self.set(ids::gain(n), 0.0, out);
                }
            },
            _ => {}
        }
        self.release(out);
        self.redraw();
    }

    /// Gives band `b` type `k`: a band changes type in place; a band made a
    /// cut moves to that cut (the band goes Unused), and a cut made a band
    /// moves to the first Unused band, at the same frequency.
    fn retype(&mut self, b: Band, k: Kind, out: &mut Vec<ChainAction>) {
        if self.kind(b) == k {
            return;
        }
        let hz = match b {
            Band::Band(n) => self.v(ids::freq(n)),
            Band::Low => self.v(ids::LOW_CUT),
            Band::High => self.v(ids::HIGH_CUT),
        };
        match (b, k.type_value()) {
            (Band::Band(n), Some(t)) => self.set(ids::band_type(n), t, out),
            (_, None) => {
                // To a cut. A cut that is already on keeps its place.
                let (cut, slope, lo, hi, to) = if k == Kind::LowCut {
                    (ids::LOW_CUT, ids::LOW_SLOPE, 20.0, 2_000.0, Band::Low)
                } else {
                    (
                        ids::HIGH_CUT,
                        ids::HIGH_SLOPE,
                        1_000.0,
                        20_000.0,
                        Band::High,
                    )
                };
                if self.v(slope) > 0.0 {
                    return;
                }
                match b {
                    Band::Band(n) => {
                        self.set(ids::band_use(n), ids::USE_UNUSED, out);
                        self.set(ids::gain(n), 0.0, out);
                    }
                    Band::Low => self.set(ids::LOW_SLOPE, 0.0, out),
                    Band::High => self.set(ids::HIGH_SLOPE, 0.0, out),
                }
                self.set(cut, hz.clamp(lo, hi), out);
                self.set(slope, 1.0, out);
                self.release(out);
                self.redraw();
                self.focus = None;
                self.pick(to, out);
            }
            (cut, Some(t)) => {
                // A cut to a band.
                let Some(n) = self.free() else {
                    return;
                };
                let slope = if cut == Band::Low {
                    ids::LOW_SLOPE
                } else {
                    ids::HIGH_SLOPE
                };
                self.set(slope, 0.0, out);
                self.set(ids::band_type(n), t, out);
                self.set(ids::freq(n), hz, out);
                self.set(ids::gain(n), 0.0, out);
                self.set(ids::band_use(n), ids::USE_ON, out);
                self.release(out);
                self.redraw();
                self.pick(Band::Band(n), out);
            }
        }
    }

    /// The finger moved.
    pub fn moved(&mut self, finger: u64, x: i32, y: i32, r: Rect, out: &mut Vec<ChainAction>) {
        let l = layout(r, self);
        if let Some(p) = self.pinch.filter(|p| p.finger == finger) {
            let Some((_, Grab::Dot { band, .. })) = self.grab else {
                return;
            };
            let (dx, dy) = Self::to_screen(l.plot, self.dot(band));
            let d = (dx - x as f32).hypot(dy - y as f32).max(4.0);
            if let Band::Band(n) = band {
                // Fingers apart: wider (a lower Q); together: narrower.
                let q = (p.q0 * p.d0 / d).clamp(0.1, 18.0);
                self.set(ids::q(n), (q * 100.0).round() / 100.0, out);
                self.redraw();
            }
            self.pinch = Some(Pinch { x, y, ..p });
            return;
        }
        let Some((f, grab)) = self.grab else {
            return;
        };
        if f != finger {
            return;
        }
        if let Grab::Dot {
            band,
            x0,
            y0,
            moved,
        } = grab
        {
            let moved = moved || (x - x0).abs().max((y - y0).abs()) >= LIFT;
            self.grab = Some((
                finger,
                Grab::Dot {
                    band,
                    x0,
                    y0,
                    moved,
                },
            ));
            if moved {
                let fx = (x - l.plot.x) as f32 / l.plot.w.max(1) as f32;
                let fy = (l.plot.bottom() - y) as f32 / l.plot.h.max(1) as f32;
                self.drag_to(band, fx, fy, out);
            }
        }
    }

    /// The finger lifted (`cancel`: the touch was taken away).
    pub fn up(
        &mut self,
        finger: u64,
        x: i32,
        y: i32,
        r: Rect,
        cancel: bool,
        out: &mut Vec<ChainAction>,
    ) {
        if self.pinch.is_some_and(|p| p.finger == finger) {
            self.pinch = None;
            return;
        }
        let Some((f, grab)) = self.grab else {
            return;
        };
        if f != finger {
            return;
        }
        self.grab = None;
        self.pinch = None;
        match grab {
            Grab::Graph { x: gx, y: gy } if !cancel => {
                let near = (x - gx).abs().max((y - gy).abs()) < LIFT;
                let second = self
                    .tapped
                    .is_some_and(|(tx, ty)| (tx - gx).abs().max((ty - gy).abs()) <= 24);
                if near && second {
                    self.tapped = None;
                    let l = layout(r, self);
                    let fx = (gx - l.plot.x) as f32 / l.plot.w.max(1) as f32;
                    let fy = (l.plot.bottom() - gy) as f32 / l.plot.h.max(1) as f32;
                    self.add(x_freq(fx), ((fy - 0.5) * 2.0 * DB).clamp(-DB, DB), out);
                } else if near {
                    self.tapped = Some((gx, gy));
                } else {
                    self.tapped = None;
                }
            }
            _ => {}
        }
        self.release(out);
    }

    /// The dot held and lifted, for the readout.
    fn held(&self) -> Option<Band> {
        match self.grab {
            Some((_, Grab::Dot { band, moved, .. })) if moved || self.pinch.is_some() => Some(band),
            _ => None,
        }
    }

    /// The text of value `id` as the EQ shows it ("320 Hz", "-3.5 dB").
    fn text(&self, id: u16) -> String {
        let v = self.v(id);
        match Band::of(id) {
            _ if id == ids::LOW_SLOPE || id == ids::HIGH_SLOPE => {
                format!("{} dB/oct", if v >= 2.0 { 24 } else { 12 })
            }
            _ if id == ids::LOW_CUT || id == ids::HIGH_CUT => hz_text(v),
            Some(Band::Band(n)) if id == ids::freq(n) => hz_text(v),
            Some(Band::Band(n)) if id == ids::gain(n) => db_text(v),
            _ => format!("{v:.2}"),
        }
    }

    /// The readout over a held dot.
    pub fn readout(&self, b: Band) -> String {
        match b {
            Band::Low => format!(
                "{} · {}",
                self.text(ids::LOW_CUT),
                self.text(ids::LOW_SLOPE)
            ),
            Band::High => format!(
                "{} · {}",
                self.text(ids::HIGH_CUT),
                self.text(ids::HIGH_SLOPE)
            ),
            Band::Band(n) if self.kind(b) == Kind::Notch => {
                format!("{} · Q {:.2}", self.text(ids::freq(n)), self.v(ids::q(n)))
            }
            Band::Band(n) => format!(
                "{} · {} · Q {:.2}",
                self.text(ids::freq(n)),
                self.text(ids::gain(n)),
                self.v(ids::q(n))
            ),
        }
    }

    /// The three value fields for band `b`: name and value.
    pub fn fields(&self, b: Band) -> [(&'static str, String, bool); 3] {
        match b {
            Band::Low => [
                ("Freq", self.text(ids::LOW_CUT), true),
                ("Slope", self.text(ids::LOW_SLOPE), true),
                ("Q", "0.71".into(), false),
            ],
            Band::High => [
                ("Freq", self.text(ids::HIGH_CUT), true),
                ("Slope", self.text(ids::HIGH_SLOPE), true),
                ("Q", "0.71".into(), false),
            ],
            Band::Band(n) => {
                let notch = self.kind(b) == Kind::Notch;
                [
                    ("Freq", self.text(ids::freq(n)), true),
                    (
                        "Gain",
                        if notch {
                            "—".into()
                        } else {
                            self.text(ids::gain(n))
                        },
                        !notch,
                    ),
                    ("Q", format!("{:.2}", self.v(ids::q(n))), true),
                ]
            }
        }
    }

    /// The panel's title line: "BAND 3 · BELL", with "BYPASSED" while off.
    pub fn title(&self, b: Band) -> String {
        let mut t = format!("{} · {}", b.name(), self.kind(b).name()).to_uppercase();
        if !self.on(b) {
            t.push_str(" · BYPASSED");
        }
        t
    }

    /// The page's targets for the wheel's outline and the audit: a dot per
    /// band in use (the wheel turns its Q), and the panel's buttons.
    pub fn targets(&self, r: Rect, out: &mut Vec<Target>) {
        let l = layout(r, self);
        for (k, b) in self.used().into_iter().enumerate() {
            let (x, y) = Self::to_screen(l.plot, self.dot(b));
            let rect = Rect::new(x as i32 - 22, y as i32 - 22, 44, 44);
            out.push(Target::drag("eq-band", k, b.name(), rect).towards(40, -30));
        }
        let picked = self.picked();
        for (k, (rect, button)) in l.buttons.iter().enumerate() {
            let (id, label, off) = match *button {
                Button::Chip(b) => ("eq-chip", b.name(), picked == Some(b)),
                Button::Add => ("eq-add", "+ Band".to_string(), false),
                Button::Kind(kind) => (
                    "eq-type",
                    kind.name().to_string(),
                    picked.is_none_or(|b| self.kind(b) == kind),
                ),
                Button::Slope => ("eq-slope", "Slope".to_string(), false),
                Button::On => (
                    "eq-on",
                    "On".to_string(),
                    !matches!(picked, Some(Band::Band(_))),
                ),
                Button::Listen => (
                    "eq-listen",
                    "Listen".to_string(),
                    !matches!(picked, Some(Band::Band(_))),
                ),
                Button::Remove => ("eq-remove", "Remove".to_string(), picked.is_none()),
            };
            out.push(Target::tap(id, k, label, *rect).disabled_if(off));
        }
    }

    /// The band behind target `index` of id "eq-band".
    pub fn band_target(&self, index: usize) -> Option<Band> {
        self.used().get(index).copied()
    }

    /// The panel button behind target `index` of an "eq-…" button id.
    pub fn button_target(&self, r: Rect, index: usize) -> Option<Button> {
        layout(r, self).buttons.get(index).map(|&(_, b)| b)
    }

    pub fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let l = layout(r, self);
        // ---- The graph.
        cx.canvas
            .fill_round_rect(l.graph, t.radius, t.bg.mix(t.surface, 0.5));
        cx.canvas.stroke_round_rect(l.graph, t.radius, 1, t.outline);
        let p = l.plot;
        for (hz, label) in [
            (20.0, "20"),
            (50.0, "50"),
            (100.0, "100"),
            (200.0, "200"),
            (500.0, "500"),
            (1_000.0, "1k"),
            (2_000.0, "2k"),
            (5_000.0, "5k"),
            (10_000.0, "10k"),
            (20_000.0, "20k"),
        ] {
            let gx = p.x + (freq_x(hz) * p.w as f32) as i32;
            let strong = matches!(label, "100" | "1k" | "10k");
            let c = if strong {
                t.outline
            } else {
                t.outline.mix(t.bg, 0.5)
            };
            cx.canvas.fill_rect(Rect::new(gx, p.y, 1, p.h), c);
            let lr = Rect::new(gx - 20, p.bottom() + 6, 40, 16);
            cx.text(FontId::Sans, 12.0, lr, Align::Center, t.text_dim, label);
        }
        for db in [18, 12, 6, 0, -6, -12, -18] {
            let gy = p.bottom() - ((0.5 + db as f32 / (2.0 * DB)) * p.h as f32) as i32;
            let c = if db == 0 {
                t.outline.mix(t.text_faint, 0.4)
            } else {
                t.outline.mix(t.bg, 0.5)
            };
            cx.canvas.fill_rect(Rect::new(p.x, gy, p.w, 1), c);
            let lr = Rect::new(l.graph.x + 4, gy - 8, 34, 16);
            let s = if db > 0 {
                format!("+{db}")
            } else {
                db.to_string()
            };
            cx.text(FontId::Sans, 12.0, lr, Align::Right, t.text_dim, &s);
        }
        let pts = |db: &[f32]| -> Vec<(f32, f32)> {
            db.iter()
                .enumerate()
                .map(|(k, &d)| {
                    let y = (0.5 + d / (2.0 * DB)).clamp(-0.02, 1.02);
                    Self::to_screen(p, (k as f32 / (K - 1) as f32, y))
                })
                .collect()
        };
        let clip = cx.canvas.clip();
        cx.canvas.set_clip(clip.intersect(p.inset_xy(-2, -2)));
        let mid = p.bottom() as f32 - 0.5 * p.h as f32;
        let picked = self.picked();
        for (b, shape) in &self.shapes {
            let mut m = Mask::new(p);
            let alpha = if Some(*b) == picked { 0.30 } else { 0.10 };
            m.fill_under(&pts(shape), mid, alpha);
            m.draw(cx.canvas, b.color());
        }
        if !self.curve.is_empty() {
            let mut m = Mask::new(p.inset_xy(-2, -2));
            m.polyline(&pts(&self.curve), 2.5);
            m.draw(cx.canvas, t.text);
        }
        cx.canvas.set_clip(clip);
        for b in self.used() {
            let (x, y) = Self::to_screen(p, self.dot(b));
            let sel = Some(b) == picked;
            let rad = if sel { 13.0 } else { 11.0 };
            if sel {
                let mut ring = Mask::new(l.graph);
                ring.disc(x, y, rad + 3.0);
                ring.draw(cx.canvas, t.text);
            }
            let mut m = Mask::new(l.graph);
            m.disc(x, y, rad);
            let fill = if self.on(b) { b.color() } else { t.text_faint };
            m.draw(cx.canvas, fill);
            let lr = Rect::new(x as i32 - 14, y as i32 - 9, 28, 18);
            cx.text(FontId::Bold, 12.0, lr, Align::Center, t.bg, &b.label());
        }
        if let Some(b) = self.held() {
            let (x, y) = Self::to_screen(p, self.dot(b));
            let s = self.readout(b);
            let w = cx.measure(FontId::Bold, t.size_small - 1.0, &s) + 24;
            let bx = (x as i32 - w / 2).clamp(p.x, p.right() - w);
            let by = if y as i32 - p.y > 64 {
                y as i32 - 54
            } else {
                y as i32 + 26
            };
            let bubble = Rect::new(bx, by, w, 30);
            cx.canvas.fill_round_rect(bubble, 8, t.raised);
            cx.canvas.stroke_round_rect(bubble, 8, 1, t.outline);
            cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                bubble,
                Align::Center,
                t.text,
                &s,
            );
        }
        // ---- The band panel.
        cx.panel(l.panel);
        for &(rect, button) in &l.buttons {
            self.paint_button(cx, rect, button, picked);
        }
        let Some(b) = picked else {
            cx.text(
                FontId::Sans,
                t.size_small,
                l.title,
                Align::Left,
                t.text_dim,
                "No bands yet: + Band, or tap the graph twice where you want one",
            );
            return;
        };
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            l.title,
            Align::Left,
            b.color(),
            &self.title(b),
        );
        for (rect, (name, value, live)) in l.fields.iter().zip(self.fields(b)) {
            cx.canvas
                .fill_round_rect(*rect, 8, t.bg.mix(t.surface, 0.5));
            let inner = rect.inset_xy(12, 0);
            let ink = if live { t.text } else { t.text_faint };
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                inner,
                Align::Left,
                t.text_dim,
                name,
            );
            cx.text(FontId::Bold, t.size_small, inner, Align::Right, ink, &value);
        }
    }

    fn paint_button(&self, cx: &mut PaintCx, rect: Rect, button: Button, picked: Option<Band>) {
        let t = cx.theme.clone();
        match button {
            Button::Chip(b) => {
                let sel = picked == Some(b);
                cx.canvas.fill_round_rect(
                    rect,
                    8,
                    if sel {
                        t.raised
                    } else {
                        t.surface.mix(t.raised, 0.4)
                    },
                );
                if sel {
                    cx.canvas.stroke_round_rect(rect, 8, 2, b.color());
                }
                let dot = Rect::new(rect.x + 8, rect.y + (rect.h - 22) / 2, 22, 22);
                let mut m = Mask::new(dot.inset(-1));
                m.disc(dot.x as f32 + 11.0, dot.y as f32 + 11.0, 11.0);
                m.draw(cx.canvas, if self.on(b) { b.color() } else { t.text_faint });
                cx.text(FontId::Bold, 11.0, dot, Align::Center, t.bg, &b.label());
                let text = Rect::new(
                    dot.right() + 6,
                    rect.y,
                    rect.right() - dot.right() - 8,
                    rect.h,
                );
                let kind = match b {
                    Band::Band(_) => self.kind(b).name(),
                    _ => "Cut",
                };
                let ink = if self.on(b) { t.text } else { t.text_dim };
                cx.text(FontId::Sans, 12.0, text, Align::Left, ink, kind);
            }
            Button::Add => {
                cx.canvas.stroke_round_rect(rect, 8, 1, t.outline);
                cx.text(
                    FontId::Bold,
                    t.size_small - 2.0,
                    rect,
                    Align::Center,
                    t.text,
                    "+ Band",
                );
            }
            Button::Kind(k) => {
                let lit = picked.is_some_and(|b| self.kind(b) == k);
                let (fill, ink) = if lit {
                    (t.accent, t.on_accent)
                } else {
                    (t.raised, if picked.is_some() { t.text } else { t.text_dim })
                };
                cx.canvas.fill_round_rect(rect, 8, fill);
                cx.text(
                    FontId::Bold,
                    t.size_small - 3.0,
                    rect,
                    Align::Center,
                    ink,
                    k.name(),
                );
            }
            // The Slope field is drawn as a field; the button is its tap.
            Button::Slope => {}
            Button::On => {
                let band = matches!(picked, Some(Band::Band(_)));
                let on = picked.is_some_and(|b| self.on(b));
                let (fill, ink, s) = match (band, on) {
                    (true, true) => (t.good.mix(t.bg, 0.55), t.text, "ON"),
                    (true, false) => (t.raised, t.text_dim, "OFF"),
                    _ => (t.surface, t.text_dim, "ON"),
                };
                cx.canvas.fill_round_rect(rect, rect.h / 2, fill);
                cx.text(
                    FontId::Bold,
                    t.size_small - 2.0,
                    rect,
                    Align::Center,
                    ink,
                    s,
                );
            }
            Button::Listen => {
                let band = match picked {
                    Some(Band::Band(n)) => Some(n),
                    _ => None,
                };
                let lit = band.is_some() && band == self.listening();
                let (fill, ink) = match (band, lit) {
                    (_, true) => (t.accent, t.on_accent),
                    (Some(_), false) => (t.raised, t.text),
                    _ => (t.surface, t.text_dim),
                };
                cx.canvas.fill_round_rect(rect, 8, fill);
                cx.text(
                    FontId::Bold,
                    t.size_small - 2.0,
                    rect,
                    Align::Center,
                    ink,
                    "Listen",
                );
            }
            Button::Remove => {
                cx.canvas.fill_round_rect(rect, 8, t.raised);
                let ink = if picked.is_some() { t.text } else { t.text_dim };
                cx.text(
                    FontId::Bold,
                    t.size_small - 2.0,
                    rect,
                    Align::Center,
                    ink,
                    "Remove",
                );
            }
        }
    }
}

/// "80 Hz", "1.20 kHz", "12.0 kHz".
pub fn hz_text(hz: f32) -> String {
    if hz >= 10_000.0 {
        format!("{:.1} kHz", hz / 1000.0)
    } else if hz >= 1000.0 {
        format!("{:.2} kHz", hz / 1000.0)
    } else {
        format!("{} Hz", hz.round() as i32)
    }
}

/// "+3.5 dB", "0.0 dB".
pub fn db_text(db: f32) -> String {
    if db > 0.0 {
        format!("+{db:.1} dB")
    } else {
        format!("{db:.1} dB")
    }
}
