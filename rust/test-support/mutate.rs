//! Deterministic input mutation for robustness ("fuzz") tests, shared by the
//! crates' `tests/robust_*.rs` through `#[path]`, so it needs no crate and no
//! nightly toolchain:
//!
//!     #[path = "../../test-support/mutate.rs"]
//!     mod mutate;
//!
//! [`Mutator::mutate`] takes a valid input and damages it the ways files and
//! wires get damaged: flipped bits, bytes set to edge values (0, 0x7F, 0x80,
//! 0xFF), lengths and counts made huge, cut short, grown, bytes inserted,
//! removed or repeated. A parser's test feeds it thousands of these and checks
//! that every one is answered (an error or a value that keeps its promises),
//! never a panic, a hang or a runaway allocation. The same seed gives the
//! same inputs on every machine, so a failure names its seed and case.

#![allow(dead_code)]

/// A small deterministic generator (an LCG; quality is not the point).
pub struct Mutator(u64);

impl Mutator {
    pub fn new(seed: u64) -> Mutator {
        Mutator(seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1))
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }

    /// 0..n (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub fn byte(&mut self) -> u8 {
        self.next() as u8
    }

    /// Random bytes, `len` of them.
    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.byte()).collect()
    }

    /// `input` damaged by 1 to 4 mutations.
    pub fn mutate(&mut self, input: &[u8]) -> Vec<u8> {
        let mut v = input.to_vec();
        for _ in 0..1 + self.below(4) {
            self.once(&mut v);
        }
        v
    }

    fn once(&mut self, v: &mut Vec<u8>) {
        if v.is_empty() {
            v.push(self.byte());
            return;
        }
        let at = self.below(v.len());
        match self.below(9) {
            0 => v[at] ^= 1 << self.below(8),
            1 => v[at] = [0x00, 0x01, 0x7F, 0x80, 0xFE, 0xFF][self.below(6)],
            2 => v[at] = self.byte(),
            // A 16- or 32-bit field set to an edge value, either byte order.
            3 => {
                let edge: u32 = [
                    0,
                    1,
                    0x7FFF,
                    0x8000,
                    0xFFFF,
                    0x7FFF_FFFF,
                    0x8000_0000,
                    0xFFFF_FFFF,
                ][self.below(8)];
                let width = if self.below(2) == 0 { 2 } else { 4 };
                let bytes = if self.below(2) == 0 {
                    edge.to_le_bytes()
                } else {
                    edge.to_be_bytes()
                };
                for k in 0..width {
                    if let Some(b) = v.get_mut(at + k) {
                        *b = bytes[k];
                    }
                }
            }
            4 => v.truncate(at),
            5 => {
                let n = 1 + self.below(16);
                let extra = self.bytes(n);
                v.splice(at..at, extra);
            }
            6 => {
                let n = (1 + self.below(16)).min(v.len() - at);
                v.drain(at..at + n);
            }
            // Repeat a run (a chunk or a record seen twice).
            7 => {
                let n = (1 + self.below(64)).min(v.len() - at);
                let run = v[at..at + n].to_vec();
                v.splice(at..at, run);
            }
            _ => {
                let n = self.below(64);
                let extra = self.bytes(n);
                v.extend(extra);
            }
        }
    }

    /// A short string of text made from `alphabet`'s characters and random
    /// bytes, for text formats.
    pub fn text(&mut self, alphabet: &str, len: usize) -> String {
        let chars: Vec<char> = alphabet.chars().collect();
        (0..len)
            .map(|_| {
                if self.below(16) == 0 {
                    char::from(self.byte())
                } else {
                    chars[self.below(chars.len())]
                }
            })
            .collect()
    }

    /// `text` damaged by mutations, kept as valid UTF-8 (bad bytes become U+FFFD).
    pub fn mutate_text(&mut self, text: &str) -> String {
        String::from_utf8_lossy(&self.mutate(text.as_bytes())).into_owned()
    }
}

#[test]
fn the_same_seed_gives_the_same_inputs() {
    let (mut a, mut b) = (Mutator::new(7), Mutator::new(7));
    let seed = b"RIFF....WAVEfmt ";
    for _ in 0..100 {
        assert_eq!(a.mutate(seed), b.mutate(seed));
    }
    assert_ne!(Mutator::new(8).mutate(seed), Mutator::new(7).mutate(seed));
}
