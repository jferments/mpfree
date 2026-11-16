//! The control channel between the host and the plugin process (a Unix
//! socket, control thread only): length-prefixed messages, little-endian.
//!
//! A message is `u32 length`, then `u8 tag`, then the fields. Strings and
//! byte strings are `u32 length` then the bytes.

use mpfree_clap::{CurveSpec, ParamSpec};
use mpfree_node::Kind;
use std::io::{self, Read, Write};

/// Messages the host sends.
#[derive(Clone, Debug, PartialEq)]
pub enum ToChild {
    Prepare {
        rate: u32,
        max_block: u32,
    },
    SaveState,
    LoadState(Vec<u8>),
    ParamText {
        index: u16,
        value: f32,
    },
    Quit,
    /// Load one of the plugin's presets: a file (its path's bytes) or one
    /// inside the plugin (None), and its load key.
    LoadPreset {
        file: Option<Vec<u8>>,
        key: Option<String>,
    },
    /// Every parameter's value and text, as the plugin holds them now (an
    /// MPC plugin's skin shows these).
    Values,
}

/// What the plugin process says about its plugin, once, at the start.
#[derive(Clone, Debug, PartialEq)]
pub struct Hello {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub params: Vec<ParamSpec>,
}

/// Messages the plugin process sends.
#[derive(Clone, Debug, PartialEq)]
pub enum ToHost {
    Hello(Hello),
    Prepared {
        ok: bool,
        latency: u32,
        tail: u32,
    },
    State(Option<Vec<u8>>),
    Loaded(bool),
    Text(Option<String>),
    /// The answer to `LoadPreset`: done, or the plugin's reason.
    PresetLoaded(Result<(), String>),
    /// The answer to `Values`: each parameter's value (0 to 1) and text,
    /// or None from a plugin that can't say (CLAP's go through the
    /// node).
    Values(Option<(Vec<f32>, Vec<String>)>),
    /// The plugin couldn't be loaded or made; the process exits after this.
    Failed(String),
}

/// The largest message either side accepts (state can be big).
pub const MAX_MESSAGE: usize = 64 << 20;

struct Enc(Vec<u8>);

impl Enc {
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn f32(&mut self, v: f32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
        self
    }
    fn str(&mut self, s: &str) -> &mut Self {
        self.bytes(s.as_bytes())
    }
}

struct Dec<'a>(&'a [u8]);

fn bad() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "bad plughost message")
}

impl Dec<'_> {
    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        if self.0.len() < n {
            return Err(bad());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().map_err(|_| bad())?,
        ))
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| bad())?,
        ))
    }
    fn f32(&mut self) -> io::Result<f32> {
        Ok(f32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| bad())?,
        ))
    }
    fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn str(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?).map_err(|_| bad())
    }
}

fn frame(w: &mut impl Write, body: &[u8]) -> io::Result<()> {
    let mut out = Vec::with_capacity(body.len() + 4);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    w.write_all(&out)?;
    w.flush()
}

fn read_frame(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let n = u32::from_le_bytes(len) as usize;
    if n == 0 || n > MAX_MESSAGE {
        return Err(bad());
    }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body)?;
    Ok(body)
}

pub fn send_to_child(w: &mut impl Write, m: &ToChild) -> io::Result<()> {
    let mut e = Enc(Vec::new());
    match m {
        ToChild::Prepare { rate, max_block } => {
            e.u8(1).u32(*rate).u32(*max_block);
        }
        ToChild::SaveState => {
            e.u8(2);
        }
        ToChild::LoadState(b) => {
            e.u8(3).bytes(b);
        }
        ToChild::ParamText { index, value } => {
            e.u8(4).u16(*index).f32(*value);
        }
        ToChild::Quit => {
            e.u8(5);
        }
        ToChild::LoadPreset { file, key } => {
            e.u8(6)
                .u8(u8::from(file.is_some()))
                .bytes(file.as_deref().unwrap_or(&[]))
                .u8(u8::from(key.is_some()))
                .str(key.as_deref().unwrap_or(""));
        }
        ToChild::Values => {
            e.u8(7);
        }
    }
    frame(w, &e.0)
}

pub fn read_to_child(r: &mut impl Read) -> io::Result<ToChild> {
    let body = read_frame(r)?;
    let mut d = Dec(&body);
    Ok(match d.u8()? {
        1 => ToChild::Prepare {
            rate: d.u32()?,
            max_block: d.u32()?,
        },
        2 => ToChild::SaveState,
        3 => ToChild::LoadState(d.bytes()?),
        4 => ToChild::ParamText {
            index: d.u16()?,
            value: d.f32()?,
        },
        5 => ToChild::Quit,
        6 => {
            let some = d.u8()? != 0;
            let file = d.bytes()?;
            let has_key = d.u8()? != 0;
            let key = d.str()?;
            ToChild::LoadPreset {
                file: some.then_some(file),
                key: has_key.then_some(key),
            }
        }
        7 => ToChild::Values,
        _ => return Err(bad()),
    })
}

fn put_param(e: &mut Enc, p: &ParamSpec) {
    e.str(&p.name)
        .str(&p.group)
        .f32(p.min)
        .f32(p.max)
        .f32(p.default)
        .u8(u8::from(p.readonly));
    match &p.curve {
        CurveSpec::Linear => {
            e.u8(0);
        }
        CurveSpec::Stepped => {
            e.u8(1);
        }
        CurveSpec::Toggle => {
            e.u8(2);
        }
        CurveSpec::Choice(names) => {
            e.u8(3).u32(names.len() as u32);
            for n in names {
                e.str(n);
            }
        }
    }
}

fn get_param(d: &mut Dec) -> io::Result<ParamSpec> {
    let (name, group) = (d.str()?, d.str()?);
    let (min, max, default) = (d.f32()?, d.f32()?, d.f32()?);
    let readonly = d.u8()? != 0;
    let curve = match d.u8()? {
        0 => CurveSpec::Linear,
        1 => CurveSpec::Stepped,
        2 => CurveSpec::Toggle,
        3 => {
            let n = d.u32()? as usize;
            if n > 4096 {
                return Err(bad());
            }
            CurveSpec::Choice((0..n).map(|_| d.str()).collect::<io::Result<_>>()?)
        }
        _ => return Err(bad()),
    };
    Ok(ParamSpec {
        name,
        group,
        min,
        max,
        default,
        curve,
        readonly,
    })
}

pub fn send_to_host(w: &mut impl Write, m: &ToHost) -> io::Result<()> {
    let mut e = Enc(Vec::new());
    match m {
        ToHost::Hello(h) => {
            e.u8(101)
                .str(&h.id)
                .str(&h.name)
                .u8(u8::from(h.kind == Kind::Instrument))
                .u32(h.params.len() as u32);
            for p in &h.params {
                put_param(&mut e, p);
            }
        }
        ToHost::Prepared { ok, latency, tail } => {
            e.u8(102).u8(u8::from(*ok)).u32(*latency).u32(*tail);
        }
        ToHost::State(s) => {
            e.u8(103)
                .u8(u8::from(s.is_some()))
                .bytes(s.as_deref().unwrap_or(&[]));
        }
        ToHost::Loaded(ok) => {
            e.u8(104).u8(u8::from(*ok));
        }
        ToHost::Text(t) => {
            e.u8(105)
                .u8(u8::from(t.is_some()))
                .str(t.as_deref().unwrap_or(""));
        }
        ToHost::PresetLoaded(r) => {
            e.u8(106)
                .u8(u8::from(r.is_ok()))
                .str(r.as_ref().err().map_or("", String::as_str));
        }
        ToHost::Values(v) => {
            e.u8(107).u8(u8::from(v.is_some()));
            if let Some((values, texts)) = v {
                e.u32(values.len() as u32);
                for (&v, t) in values.iter().zip(texts) {
                    e.f32(v).str(t);
                }
            }
        }
        ToHost::Failed(msg) => {
            e.u8(199).str(msg);
        }
    }
    frame(w, &e.0)
}

pub fn read_to_host(r: &mut impl Read) -> io::Result<ToHost> {
    let body = read_frame(r)?;
    let mut d = Dec(&body);
    Ok(match d.u8()? {
        101 => {
            let (id, name) = (d.str()?, d.str()?);
            let kind = if d.u8()? != 0 {
                Kind::Instrument
            } else {
                Kind::Effect
            };
            let n = d.u32()? as usize;
            if n > usize::from(u16::MAX) {
                return Err(bad());
            }
            let params = (0..n)
                .map(|_| get_param(&mut d))
                .collect::<io::Result<_>>()?;
            ToHost::Hello(Hello {
                id,
                name,
                kind,
                params,
            })
        }
        102 => ToHost::Prepared {
            ok: d.u8()? != 0,
            latency: d.u32()?,
            tail: d.u32()?,
        },
        103 => {
            let some = d.u8()? != 0;
            let b = d.bytes()?;
            ToHost::State(some.then_some(b))
        }
        104 => ToHost::Loaded(d.u8()? != 0),
        105 => {
            let some = d.u8()? != 0;
            let s = d.str()?;
            ToHost::Text(some.then_some(s))
        }
        106 => {
            let ok = d.u8()? != 0;
            let why = d.str()?;
            ToHost::PresetLoaded(if ok { Ok(()) } else { Err(why) })
        }
        107 => {
            if d.u8()? == 0 {
                ToHost::Values(None)
            } else {
                let n = d.u32()? as usize;
                if n > usize::from(u16::MAX) {
                    return Err(bad());
                }
                let mut values = Vec::with_capacity(n);
                let mut texts = Vec::with_capacity(n);
                for _ in 0..n {
                    values.push(d.f32()?);
                    texts.push(d.str()?);
                }
                ToHost::Values(Some((values, texts)))
            }
        }
        199 => ToHost::Failed(d.str()?),
        _ => return Err(bad()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_round_trips() {
        let to_child = [
            ToChild::Prepare {
                rate: 44_100,
                max_block: 256,
            },
            ToChild::SaveState,
            ToChild::LoadState(vec![1, 2, 3]),
            ToChild::ParamText {
                index: 7,
                value: -3.5,
            },
            ToChild::Quit,
            ToChild::LoadPreset {
                file: Some(b"/data/Pads/Warm.fxp".to_vec()),
                key: None,
            },
            ToChild::LoadPreset {
                file: None,
                key: Some("bright".into()),
            },
            ToChild::Values,
        ];
        for m in &to_child {
            let mut buf = Vec::new();
            send_to_child(&mut buf, m).unwrap();
            assert_eq!(&read_to_child(&mut buf.as_slice()).unwrap(), m);
        }
        let hello = Hello {
            id: "org.x".into(),
            name: "X".into(),
            kind: Kind::Effect,
            params: vec![
                ParamSpec {
                    name: "A".into(),
                    group: "G".into(),
                    min: 0.0,
                    max: 2.0,
                    default: 1.0,
                    curve: CurveSpec::Choice(vec!["a".into(), "b".into(), "c".into()]),
                    readonly: false,
                },
                ParamSpec {
                    name: "B".into(),
                    group: "".into(),
                    min: -1.0,
                    max: 1.0,
                    default: 0.0,
                    curve: CurveSpec::Linear,
                    readonly: true,
                },
            ],
        };
        let to_host = [
            ToHost::Hello(hello),
            ToHost::Prepared {
                ok: true,
                latency: 3,
                tail: 900,
            },
            ToHost::State(Some(vec![9; 10])),
            ToHost::State(None),
            ToHost::Loaded(false),
            ToHost::Text(Some("Saw".into())),
            ToHost::Text(None),
            ToHost::PresetLoaded(Ok(())),
            ToHost::PresetLoaded(Err("no such preset".into())),
            ToHost::Values(Some((vec![0.25, 1.0], vec!["25%".into(), "On".into()]))),
            ToHost::Values(None),
            ToHost::Failed("no".into()),
        ];
        for m in &to_host {
            let mut buf = Vec::new();
            send_to_host(&mut buf, m).unwrap();
            assert_eq!(&read_to_host(&mut buf.as_slice()).unwrap(), m);
        }
    }

    #[test]
    fn junk_is_refused() {
        assert!(read_to_host(&mut [0u8, 0, 0, 0].as_slice()).is_err());
        assert!(read_to_host(&mut [2u8, 0, 0, 0, 101, 9].as_slice()).is_err());
        assert!(read_to_child(&mut [1u8, 0, 0, 0, 77].as_slice()).is_err());
    }
}
