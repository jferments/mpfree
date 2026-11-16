//! A skin read from `TUI.json`: its custom components, tabs and replacement
//! strings, as the SDK's `docs/ui-reference.md` describes them. Reading is
//! forgiving where the MPC is strict (a missing key takes its default), so a
//! skin that loads on the MPC always loads here, and one that has a small
//! mistake still draws.

use crate::json::{self, Json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Files bigger than this are refused (the biggest catalog TUI.json is about
/// 9 MB).
pub const MAX_FILE: u64 = 64 << 20;
/// Imports nest at most this deep.
const MAX_IMPORT_DEPTH: usize = 8;

/// A rectangle in a parent's pixels (fractions allowed).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> RectF {
        RectF { x, y, w, h }
    }

    /// `"x y w h"` with any of space, comma, semicolon, tab or newline between.
    pub fn parse(s: &str) -> Option<RectF> {
        let v: Vec<f32> = s
            .split([' ', ',', ';', '\t', '\n'])
            .filter(|p| !p.is_empty())
            .map(|p| p.parse::<f32>().ok().filter(|f| f.is_finite()))
            .collect::<Option<Vec<f32>>>()?;
        (v.len() == 4).then(|| RectF::new(v[0], v[1], v[2], v[3]))
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    pub fn offset(&self, dx: f32, dy: f32) -> RectF {
        RectF::new(self.x + dx, self.y + dy, self.w, self.h)
    }
}

/// An `AARRGGBB` colour as the MPC writes it ("0" and "" are transparent),
/// as straight (not premultiplied) ARGB.
pub fn colour(s: &str) -> u32 {
    let s = s.trim().trim_start_matches('#');
    if s.is_empty() || s == "0" {
        return 0;
    }
    let v = u32::from_str_radix(s, 16).unwrap_or(0);
    if s.len() <= 6 {
        0xFF00_0000 | v // RRGGBB without alpha
    } else {
        v
    }
}

/// Handle names inside a child mapped to names in its parent's scope.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Remap(pub Vec<(String, String)>);

impl Remap {
    fn read(j: &Json) -> Remap {
        Remap(
            j.key_values()
                .filter_map(|(k, v)| Some((k.to_string(), v.as_str()?.to_string())))
                .collect(),
        )
    }

    /// `h` in the parent's scope: a literal match, else the first `*`
    /// pattern that matches (the `*` carries what it matched), else `h`.
    pub fn apply(&self, h: &str) -> String {
        if let Some((_, v)) = self.0.iter().find(|(k, _)| k == h) {
            return v.clone();
        }
        for (k, v) in &self.0 {
            if let Some(star) = k.find('*') {
                let (pre, post) = (&k[..star], &k[star + 1..]);
                if h.len() >= pre.len() + post.len() && h.starts_with(pre) && h.ends_with(post) {
                    let mid = &h[pre.len()..h.len() - post.len()];
                    return v.replacen('*', mid, 1);
                }
            }
        }
        h.to_string()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Puts copy `r`'s number in each `{Increment:Base=n,Step=s,Offset=o}`.
pub fn increment(h: &str, r: usize) -> String {
    let mut out = String::new();
    let mut rest = h;
    while let Some(i) = rest.find("{Increment:") {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let args = &rest[i + "{Increment:".len()..i + j];
        let (mut base, mut step, mut offset) = (0i64, 1i64, 0i64);
        for kv in args.split(',') {
            let (k, v) = kv.split_once('=').unwrap_or((kv, "0"));
            let v: i64 = v.trim().parse().unwrap_or(0);
            match k.trim() {
                "Base" => base = v,
                "Step" => step = v,
                "Offset" => offset = v,
                _ => {}
            }
        }
        out.push_str(&(base + r as i64 * step + offset).max(0).to_string());
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnAction {
    Invalid,
    MouseDown,
    DoubleClick,
    EnterPressed,
    FocusLost,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Handler {
    QLink,
    ShowOverlay,
    ToggleSwitch,
    FocusLost,
    Other(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub on: OnAction,
    pub handler: Handler,
    pub handle: String,
    pub data: String,
    pub remap: Remap,
}

impl Action {
    fn read(j: &Json) -> Action {
        Action {
            on: match j.at("onAction").str_or_empty() {
                "Mouse Down" => OnAction::MouseDown,
                "Double Click" => OnAction::DoubleClick,
                "Enter Pressed" => OnAction::EnterPressed,
                "Focus Lost" => OnAction::FocusLost,
                _ => OnAction::Invalid,
            },
            handler: match j.at("handler").str_or_empty() {
                "Q-Link" => Handler::QLink,
                "Show Overlay" => Handler::ShowOverlay,
                "Toggle Switch" => Handler::ToggleSwitch,
                "Focus Lost" => Handler::FocusLost,
                s => Handler::Other(s.to_string()),
            },
            handle: j.at("handleName").str_or_empty().to_string(),
            data: j.at("additionalData").str_or_empty().to_string(),
            remap: Remap::read(j.at("handle remapping").at("map")),
        }
    }
}

/// A colour filled first, then an image over it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fill {
    pub colour: u32,
    pub image: String,
}

impl Fill {
    fn read(j: &Json) -> Fill {
        Fill {
            colour: colour(j.at("colour").str_or_empty()),
            image: j.at("image").str_or_empty().to_string(),
        }
    }
}

/// How text is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub font: String,
    pub style: String,
    /// The font's height in pixels (ascent plus descent, as JUCE means it).
    pub height: f32,
    pub colour: u32,
    pub h_align: HAlign,
    pub v_align: VAlign,
    pub case: Case,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HAlign {
    Left,
    Centre,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VAlign {
    Top,
    Centre,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    Original,
    Upper,
    Lower,
}

impl TextStyle {
    pub fn read(j: &Json) -> TextStyle {
        let font = j.at("font");
        let just = j.at("justification").str_or_empty();
        let has = |f: &str| just.split_whitespace().any(|w| w == f);
        TextStyle {
            font: font
                .at("name")
                .as_str()
                .unwrap_or("Titillium Web")
                .to_string(),
            style: font.at("style").as_str().unwrap_or("Regular").to_string(),
            height: font.at("height").as_f64().unwrap_or(20.0).clamp(1.0, 400.0) as f32,
            colour: j
                .get("colour")
                .map(|c| colour(c.str_or_empty()))
                .unwrap_or(0xFFFF_FFFF),
            h_align: if has("horizontallyCentred") || has("centred") {
                HAlign::Centre
            } else if has("right") {
                HAlign::Right
            } else {
                HAlign::Left
            },
            v_align: if has("top") {
                VAlign::Top
            } else if has("bottom") {
                VAlign::Bottom
            } else {
                VAlign::Centre
            },
            case: match j.at("case").str_or_empty() {
                "Upper Case" => Case::Upper,
                "Lower Case" => Case::Lower,
                _ => Case::Original,
            },
        }
    }

    pub fn apply_case(&self, s: &str) -> String {
        match self.case {
            Case::Original => s.to_string(),
            Case::Upper => s.to_uppercase(),
            Case::Lower => s.to_lowercase(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drag {
    Vertical,
    Horizontal,
    Rotary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelKind {
    Name,
    Value,
    Text,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SliderKind {
    Generic,
    ColourBar {
        vertical: bool,
        background: u32,
        outline: u32,
        value: u32,
        bipolar: bool,
    },
    TrackAndThumb {
        vertical: bool,
        thumb: String,
        track: bool,
        revealed: String,
        /// 0 normal, 1 inverted, 2 bipolar.
        reveal: u8,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum KnobKind {
    FilmStrip {
        strip: String,
        frames: u32,
        invert: bool,
    },
    ValueSlider(TextStyle),
    Generic {
        label: bool,
        bipolar: bool,
    },
    Rotating {
        foreground: String,
        background: String,
        scale: f32,
    },
}

/// The overlay looks of a `KnobOverlay`.
#[derive(Clone, Debug, PartialEq)]
pub struct KnobOverlay {
    pub style: String,
    pub bipolar: bool,
    pub handle: String,
}

/// What an entry draws.
#[derive(Clone, Debug, PartialEq)]
pub enum Widget {
    /// A custom component by name.
    Custom(String),
    Slider {
        kind: SliderKind,
        handle: String,
    },
    Knob {
        kind: KnobKind,
        drag: Drag,
        handle: String,
    },
    Label {
        style: TextStyle,
        kind: LabelKind,
        handle: String,
        text: String,
    },
    Button {
        on: String,
        off: String,
        id: u32,
        group: u32,
        handle: String,
        hold: bool,
    },
    Focus {
        background: u32,
        outline: u32,
        inset: f32,
        thickness: f32,
    },
    Image {
        colour: u32,
        image: String,
        /// Film strip frames and the handle picking the frame.
        frames: u32,
        handle: String,
        animated: bool,
    },
    Separator {
        colour: u32,
        vertical: bool,
    },
    ComboBox {
        style: TextStyle,
        handle: String,
    },
    KnobOverlay(KnobOverlay),
    MenuOverlay {
        numbers: bool,
        handle: String,
    },
    NumericOverlay {
        handle: String,
    },
    XYPad {
        x: String,
        y: String,
        image: String,
    },
    Indicator {
        on: String,
        off: String,
        id: u32,
        group: u32,
        handle: String,
    },
    HaloDisplay {
        width: f32,
        colour: u32,
        clip: u32,
        style: String,
        handle: String,
        arc: (f32, f32),
    },
    Meter {
        vertical: bool,
        invert: bool,
        inactive: String,
        peak: String,
        handle: String,
    },
    Envelope {
        name: String,
        colours: [u32; 4],
        stages: Vec<(String, String)>,
    },
    /// A type this renderer doesn't draw (or no type at all).
    Unknown(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HwFocus {
    No,
    Yes,
    Children,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhenInvalid {
    Show,
    Hide,
    HideOnInvalidName,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhenVisible {
    Always,
    WhenFocussed,
    WhenNotFocussed,
}

/// One child of a component.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub widget: Widget,
    pub remap: Remap,
    pub focus: HwFocus,
    pub invalid: WhenInvalid,
    pub visible: WhenVisible,
    pub bounds: RectF,
    /// `bounds` are fractions of the parent's size.
    pub proportional: bool,
    pub extra_handles: Vec<String>,
}

/// A custom component: a background, actions and children.
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    pub name: String,
    pub actions: Vec<Action>,
    pub focussed: Fill,
    pub unfocussed: Fill,
    pub ignore_mouse: bool,
    pub fine_wheel: bool,
    pub repeats: usize,
    pub hide_qlink_bounds: bool,
    pub children: Vec<Entry>,
    /// The folder its images are relative to.
    pub dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub name: String,
    pub key: usize,
    pub sub: usize,
    pub qlink_bounds: Vec<RectF>,
    /// The root component's name in [`Skin::components`].
    pub root: String,
    pub scale: f32,
    /// Added to every `Parameter N` on this tab (a `PartialDescription`
    /// skin's repeated tab).
    pub param_offset: usize,
}

/// A whole `TUI.json`.
#[derive(Clone, Debug, Default)]
pub struct Skin {
    pub dir: PathBuf,
    pub components: HashMap<String, Arc<Component>>,
    /// Discrete step names per final handle, such as "Parameter 7".
    pub replacements: HashMap<String, Vec<String>>,
    pub tabs: Vec<Tab>,
    /// A `PartialDescription` skin's offset (its one tab is repeated).
    pub partial: Option<usize>,
    /// Things that were wrong but didn't stop it loading.
    pub warnings: Vec<String>,
}

fn read_widget(cd: &Json) -> Widget {
    let t = cd.at("type").str_or_empty();
    let d = cd.at("data");
    let s = |k: &str| d.at(k).str_or_empty().to_string();
    let handle = || s("handleName");
    let drag = || match d.at("dragOrientation").str_or_empty() {
        "Horizontal" => Drag::Horizontal,
        "AbsoluteRotaryKnob" => Drag::Rotary,
        _ => Drag::Vertical,
    };
    let vertical = || d.at("direction").str_or_empty() != "Horizontal";
    let int = |k: &str, def: i64| d.at(k).as_i64().unwrap_or(def).clamp(0, 1 << 20) as u32;
    let float = |k: &str, def: f64| d.at(k).as_f64().unwrap_or(def) as f32;
    let col = |k: &str| colour(d.at(k).str_or_empty());
    match t {
        "Slider" => {
            let kind = match d.at("sliderType").str_or_empty() {
                "ColourBar" => SliderKind::ColourBar {
                    vertical: vertical(),
                    background: col("backgroundColour"),
                    outline: col("outlineColour"),
                    value: col("valueColour"),
                    bipolar: d.at("isBipolar").as_bool().unwrap_or(false),
                },
                "TrackAndThumb" => SliderKind::TrackAndThumb {
                    vertical: vertical(),
                    thumb: s("thumbImage"),
                    track: d.at("thumbTrackVisible").as_bool().unwrap_or(true),
                    revealed: s("revealedImage"),
                    reveal: match d.at("revealType").str_or_empty() {
                        "Inverted" => 1,
                        "Bipolar" => 2,
                        _ => 0,
                    },
                },
                _ => SliderKind::Generic,
            };
            Widget::Slider {
                kind,
                handle: handle(),
            }
        }
        "Knob" => {
            let kind = match d.at("knobType").str_or_empty() {
                "ValueSlider" => KnobKind::ValueSlider(TextStyle::read(d.at("textStyle"))),
                "GenericLegacy" | "GenericNoLabel" => KnobKind::Generic {
                    label: d.at("knobType").str_or_empty() == "GenericLegacy",
                    bipolar: d.at("polarity").str_or_empty() == "BiPolar",
                },
                "RotatingImage" => KnobKind::Rotating {
                    foreground: s("foregroundImage"),
                    background: s("backgroundImage"),
                    scale: float("imageScale", 1.0),
                },
                // FilmStrip is the enum's first value: the MPC's fallback.
                _ => KnobKind::FilmStrip {
                    strip: s("filmStrip"),
                    frames: int("numFrames", 1).max(1),
                    invert: d.at("invert").as_bool().unwrap_or(false),
                },
            };
            Widget::Knob {
                kind,
                drag: drag(),
                handle: handle(),
            }
        }
        "Label" => Widget::Label {
            style: TextStyle::read(d.at("textStyle")),
            kind: match d.at("type").str_or_empty() {
                "Value" => LabelKind::Value,
                "Text" => LabelKind::Text,
                _ => LabelKind::Name,
            },
            handle: handle(),
            text: s("text"),
        },
        "Button" => Widget::Button {
            on: s("onImage"),
            off: s("offImage"),
            id: int("buttonId", 1),
            group: int("numButtonsInGroup", 1),
            handle: handle(),
            hold: d.at("gestureBehaviour").str_or_empty() == "Hold",
        },
        "Focus" => Widget::Focus {
            background: d
                .get("backgroundColour")
                .map(|c| colour(c.str_or_empty()))
                .unwrap_or(0x19FF_FFFF),
            outline: d
                .get("outlineColour")
                .map(|c| colour(c.str_or_empty()))
                .unwrap_or(0xFFE8_1F40),
            inset: float("backgroundInset", 0.0),
            thickness: float("outlineThickness", 1.0),
        },
        "Image" => {
            let strip = d.at("imageType").str_or_empty() == "FilmStrip";
            Widget::Image {
                colour: col("colour"),
                image: s("image"),
                frames: if strip { int("numFrames", 1).max(1) } else { 1 },
                handle: if strip { handle() } else { String::new() },
                animated: strip && d.at("animated").as_bool().unwrap_or(false),
            }
        }
        "Separator" => Widget::Separator {
            colour: col("colour"),
            vertical: d.at("separatorType").as_i64().unwrap_or(0) == 0,
        },
        "ComboBox" => Widget::ComboBox {
            style: TextStyle::read(d.at("textStyle")),
            handle: handle(),
        },
        "KnobOverlay" => Widget::KnobOverlay(KnobOverlay {
            style: s("overlayType"),
            bipolar: d.at("polarityType").str_or_empty() == "Bipolar",
            handle: handle(),
        }),
        "MenuOverlay" => Widget::MenuOverlay {
            numbers: d.at("withNumbers").as_bool().unwrap_or(false),
            handle: handle(),
        },
        "NumericOverlay" | "TextEditOverlay" => Widget::NumericOverlay { handle: handle() },
        "XYPad" => Widget::XYPad {
            x: s("xPositionHandle"),
            y: s("yPositionHandle"),
            image: s("image"),
        },
        "Indicator" => Widget::Indicator {
            on: s("onImage"),
            off: s("offImage"),
            id: int("indicatorId", 1),
            group: int("numIndicatorsInGroup", 1),
            handle: handle(),
        },
        "HaloDisplay" => Widget::HaloDisplay {
            width: float("lineWidth", 5.0),
            colour: col("lineColour"),
            clip: col("clipColour"),
            style: s("style"),
            handle: handle(),
            arc: (float("arcMin", -0.67), float("arcMax", 0.67)),
        },
        "Meter" => Widget::Meter {
            vertical: !matches!(
                d.at("direction").str_or_empty(),
                "Left" | "Right" | "LeftRight"
            ),
            invert: d.at("invert").as_bool().unwrap_or(false),
            inactive: s("inactiveImage"),
            peak: s("peakImage"),
            handle: s("peakHandle"),
        },
        "Envelope" | "EnvelopeOverlay" => Widget::Envelope {
            name: s("envelopeName"),
            colours: [
                col("backgroundColour"),
                col("lineColour"),
                col("fillColour"),
                col("scaleColour"),
            ],
            stages: [
                "delayHandle",
                "attackHandle",
                "holdHandle",
                "decayHandle",
                "sustainHandle",
                "releaseHandle",
            ]
            .iter()
            .filter_map(|k| {
                let h = d.get(k)?.as_str()?;
                Some((k.trim_end_matches("Handle").to_string(), h.to_string()))
            })
            .collect(),
        },
        "" => Widget::Unknown(String::new()),
        other => Widget::Custom(other.to_string()),
    }
}

/// Built-in widget names; anything else in `type` is a custom component.
pub const BUILT_IN: &[&str] = &[
    "Slider",
    "Knob",
    "Label",
    "Button",
    "Focus",
    "Image",
    "Separator",
    "ComboBox",
    "KnobOverlay",
    "MenuOverlay",
    "NumericOverlay",
    "TextEditOverlay",
    "XYPad",
    "Indicator",
    "HaloDisplay",
    "Meter",
    "Envelope",
    "EnvelopeOverlay",
];

fn read_entry(j: &Json) -> Entry {
    let cd = j.at("componentData");
    // A legacy version-1 entry has its remap's "map" inline.
    let remap = match j.get("handle remapping") {
        Some(r) => Remap::read(r.at("map")),
        None => Remap::read(j.at("map")),
    };
    let b = j.at("bounds");
    let kind = b.at("boundsType").str_or_empty();
    let bounds = if kind == "Relative" {
        // Relative bounds name the parent's size; the catalog uses none.
        RectF::new(0.0, 0.0, 0.0, 0.0)
    } else {
        RectF::parse(b.at("bounds").str_or_empty()).unwrap_or_default()
    };
    Entry {
        name: cd.at("name").str_or_empty().to_string(),
        widget: read_widget(cd),
        remap,
        focus: match b.at("acceptsHWFocus").str_or_empty() {
            "No" => HwFocus::No,
            "Children" => HwFocus::Children,
            _ => HwFocus::Yes,
        },
        invalid: match b.at("showWhenDataModelInvalid").str_or_empty() {
            "Hide" => WhenInvalid::Hide,
            "HideOnInvalidName" => WhenInvalid::HideOnInvalidName,
            _ => WhenInvalid::Show,
        },
        visible: match b.at("whenVisible").str_or_empty() {
            "WhenFocussed" => WhenVisible::WhenFocussed,
            "WhenNotFocussed" => WhenVisible::WhenNotFocussed,
            _ => WhenVisible::Always,
        },
        bounds,
        proportional: kind == "Proportional",
        extra_handles: b
            .at("additionalInvalidatingHandles")
            .items()
            .iter()
            .filter_map(|h| h.as_str().map(str::to_string))
            .collect(),
    }
}

pub fn read_component(name: &str, j: &Json, dir: &Path) -> Component {
    let bg = j.at("backgroundData");
    Component {
        name: name.to_string(),
        actions: j.at("actions").items().iter().map(Action::read).collect(),
        // Without backgroundData the MPC fills black.
        focussed: bg.get("focussed").map(Fill::read).unwrap_or(Fill {
            colour: 0xFF00_0000,
            image: String::new(),
        }),
        unfocussed: bg.get("unfocussed").map(Fill::read).unwrap_or(Fill {
            colour: 0xFF00_0000,
            image: String::new(),
        }),
        ignore_mouse: j.at("ignoreMousePresses").as_bool().unwrap_or(false),
        fine_wheel: j.at("disableCoarseDataWheel").as_bool().unwrap_or(false),
        repeats: j.at("repeats").as_i64().unwrap_or(1).clamp(1, 256) as usize,
        hide_qlink_bounds: j.at("hideQLinkBounds").as_bool().unwrap_or(false),
        children: j
            .at("componentsData")
            .items()
            .iter()
            .map(read_entry)
            .collect(),
        dir: dir.to_path_buf(),
    }
}

/// Reads a file of at most [`MAX_FILE`] bytes as JSON.
pub fn read_json(path: &Path) -> Result<Json, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_FILE {
        return Err(format!("{} is too big", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    json::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

impl Skin {
    /// Reads `TUI.json` (or the given variant) in `dir`.
    pub fn read(path: &Path) -> Result<Skin, String> {
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let j = read_json(path)?;
        Skin::from_json(&j, &dir)
    }

    pub fn from_json(j: &Json, dir: &Path) -> Result<Skin, String> {
        let pd = j.get("pageData").ok_or("no pageData")?;
        let mut skin = Skin {
            dir: dir.to_path_buf(),
            ..Skin::default()
        };
        let cd = pd.at("componentDefinitions");
        // Imports first, so local definitions win.
        let mut seen = Vec::new();
        skin.import(cd, dir, 0, &mut seen);
        for (name, v) in cd.at("localComponentDefinitions").key_values() {
            skin.components
                .insert(name.to_string(), Arc::new(read_component(name, v, dir)));
        }
        for (k, v) in pd.at("replacementStrings").key_values() {
            let names: Vec<String> = v
                .items()
                .iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect();
            if !names.is_empty() {
                skin.replacements.insert(k.to_string(), names);
            }
        }
        let info = pd.at("info");
        if info.at("type").str_or_empty() == "PartialDescription" {
            skin.partial = info
                .at("offset")
                .as_i64()
                .filter(|&o| o > 0)
                .map(|o| o as usize);
        }
        for (i, t) in pd.at("tabs").items().iter().enumerate() {
            let root = match t.get("componentDefinition") {
                Some(c) => {
                    let name = format!("tab {i}");
                    skin.components
                        .insert(name.clone(), Arc::new(read_component(&name, c, dir)));
                    name
                }
                None => t.at("componentName").str_or_empty().to_string(),
            };
            if !skin.components.contains_key(&root) {
                skin.warnings.push(format!(
                    "tab {i} names a component that isn't defined: {root:?}"
                ));
            }
            skin.tabs.push(Tab {
                name: t.at("tabName").str_or_empty().to_string(),
                key: t.at("fnKeyIndex").as_i64().unwrap_or(0).clamp(0, 64) as usize,
                sub: t.at("fnKeySubIndex").as_i64().unwrap_or(0).clamp(0, 64) as usize,
                qlink_bounds: t
                    .at("qlinkBoundsData")
                    .items()
                    .iter()
                    .filter_map(|r| RectF::parse(r.as_str()?))
                    .collect(),
                root,
                scale: t.at("scale").as_f64().unwrap_or(1.0).clamp(0.05, 20.0) as f32,
                param_offset: 0,
            });
            if skin.tabs.len() >= 32 {
                break;
            }
        }
        if skin.tabs.is_empty() {
            return Err("no tabs".into());
        }
        Ok(skin)
    }

    /// Reads `importFiles` (paths relative to `dir`). Akai's own files under
    /// `/usr/share/Akai` are never read: MPFree draws its own overlays for
    /// the names they define ([`crate::overlay`]).
    fn import(&mut self, cd: &Json, dir: &Path, depth: usize, seen: &mut Vec<PathBuf>) {
        if depth > MAX_IMPORT_DEPTH {
            return;
        }
        for f in cd.at("importFiles").items() {
            let Some(f) = f.as_str() else { continue };
            if f.starts_with('/') || f.contains("..") {
                continue; // outside the plugin folder (Akai's own files)
            }
            let path = dir.join(f);
            if seen.contains(&path) {
                continue;
            }
            seen.push(path.clone());
            let Ok(j) = read_json(&path) else {
                continue; // a missing import contributes nothing
            };
            let idir = path.parent().unwrap_or(dir).to_path_buf();
            self.import(&j, &idir, depth + 1, seen);
            for (name, v) in j.at("localComponentDefinitions").key_values() {
                self.components
                    .insert(name.to_string(), Arc::new(read_component(name, v, &idir)));
            }
        }
    }

    /// Repeats a `PartialDescription` skin's tab to cover `params`
    /// parameters, named 1, 2, 3...
    pub fn expand_partial(&mut self, params: usize) {
        let (Some(off), Some(first)) = (self.partial, self.tabs.first().cloned()) else {
            return;
        };
        let n = params.div_ceil(off).clamp(1, 32);
        self.tabs = (0..n)
            .map(|t| Tab {
                name: (t + 1).to_string(),
                key: t,
                sub: 0,
                param_offset: t * off,
                ..first.clone()
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rects_colours_remaps_and_increments() {
        assert_eq!(
            RectF::parse("23, 45;278\t107"),
            Some(RectF::new(23.0, 45.0, 278.0, 107.0))
        );
        assert_eq!(RectF::parse("1 2 3"), None);
        assert_eq!(colour("ffe81f40"), 0xFFE8_1F40);
        assert_eq!(colour("0"), 0);
        assert_eq!(colour(""), 0);
        assert_eq!(colour("19ffffff"), 0x19FF_FFFF);
        let r = Remap(vec![
            ("Data".into(), "Parameter 2".into()),
            ("Invert/*".into(), "*".into()),
            ("*".into(), "Parameter *".into()),
        ]);
        assert_eq!(r.apply("Data"), "Parameter 2");
        assert_eq!(r.apply("Invert/Data"), "Data");
        assert_eq!(r.apply("3"), "Parameter 3");
        assert_eq!(Remap::default().apply("Parameter 9"), "Parameter 9");
        assert_eq!(
            increment("Parameter {Increment:Base=10}", 3),
            "Parameter 13"
        );
        assert_eq!(
            increment("{Increment:Base=4,Step=2,Offset=-20}/x", 1),
            "0/x"
        );
    }

    const SMALL: &str = r#"{"pageData": {"version": 1,
      "componentDefinitions": {"version": 2, "importFiles": ["/usr/share/Akai/x.json"],
        "localComponentDefinitions": [{"key": "slot", "value": {"version": 3,
          "actions": [{"version": 1, "onAction": "Mouse Down", "handler": "Q-Link", "handleName": "Data", "additionalData": ""}],
          "backgroundData": {"version": 1, "focussed": {"version": 1, "colour": "0", "image": ""}, "unfocussed": {"version": 1, "colour": "ff101010", "image": "bg.png"}},
          "ignoreMousePresses": false, "disableCoarseDataWheel": false, "repeats": 4,
          "componentsData": [{"version": 1,
            "componentData": {"version": 1, "name": "k", "type": "Knob", "data": {"version": 2, "knobType": "FilmStrip", "filmStrip": "k.png", "numFrames": 64, "invert": true, "handleName": "Data"}},
            "version": 1, "map": [{"key": "Data", "value": "Parameter {Increment:Base=2}"}],
            "bounds": {"version": 1, "acceptsHWFocus": "Yes", "showWhenDataModelInvalid": "Hide", "whenVisible": "Always", "boundsType": "Absolute", "bounds": "0 0 100 100"}}]}}]},
      "replacementStrings": [{"key": "Parameter 0", "value": ["Off", "On"]}],
      "info": {"version": 1, "type": "CompleteDescription"},
      "tabs": [{"version": 3, "tabName": "MAIN", "fnKeyIndex": 0, "fnKeySubIndex": 0, "qlinkBoundsData": ["0 0 100 400"], "componentName": "slot", "initialSize": "0 0 1280 628", "scale": 1.0},
               {"version": 3, "tabName": "BAD", "fnKeyIndex": 1, "fnKeySubIndex": 0, "qlinkBoundsData": [], "componentName": "nope", "initialSize": "0 0 1280 628", "scale": 1.0}]}}"#;

    #[test]
    fn reads_a_skin() {
        let j = json::parse(SMALL).unwrap();
        let s = Skin::from_json(&j, Path::new("/skin")).unwrap();
        assert_eq!(s.tabs.len(), 2);
        assert_eq!(s.tabs[0].name, "MAIN");
        assert_eq!(s.tabs[0].qlink_bounds, [RectF::new(0.0, 0.0, 100.0, 400.0)]);
        assert_eq!(s.warnings.len(), 1, "{:?}", s.warnings);
        assert_eq!(s.replacements["Parameter 0"], ["Off", "On"]);
        let c = &s.components["slot"];
        assert_eq!(c.repeats, 4);
        assert_eq!(c.unfocussed.colour, 0xFF10_1010);
        assert_eq!(c.unfocussed.image, "bg.png");
        assert_eq!(c.actions[0].handler, Handler::QLink);
        let e = &c.children[0];
        assert_eq!(e.remap.apply("Data"), "Parameter {Increment:Base=2}");
        assert_eq!(e.invalid, WhenInvalid::Hide);
        assert_eq!(
            e.widget,
            Widget::Knob {
                kind: KnobKind::FilmStrip {
                    strip: "k.png".into(),
                    frames: 64,
                    invert: true
                },
                drag: Drag::Vertical,
                handle: "Data".into()
            }
        );
    }

    #[test]
    fn a_partial_description_repeats_its_tab() {
        let j = json::parse(&SMALL.replace(
            r#""type": "CompleteDescription""#,
            r#""type": "PartialDescription", "offset": 16"#,
        ))
        .unwrap();
        let mut s = Skin::from_json(&j, Path::new("/skin")).unwrap();
        s.expand_partial(40);
        assert_eq!(s.tabs.len(), 3);
        assert_eq!(s.tabs[2].name, "3");
        assert_eq!(s.tabs[2].param_offset, 32);
    }
}
