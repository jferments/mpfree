//! [`Profiled`]: any node, timed.

use mpfree_node::{Ctx, Node, NodeInfo, ParamInfo, Timed};

use crate::{Probe, Span, Summary};

/// Wraps a [`Node`] and times every `process` call into a [`Probe`]. It is a
/// node itself, so it goes anywhere the wrapped node went (a chain slot, a
/// track). The cost of the wrapper is two clock reads and two thread-local
/// reads per block.
pub struct Profiled<N: Node> {
    inner: N,
    probe: Probe,
}

impl<N: Node> Profiled<N> {
    /// `limit_ns`: a block slower than this counts as over (the node's
    /// budget, or the period).
    pub fn new(inner: N, limit_ns: u64) -> Profiled<N> {
        Profiled {
            inner,
            probe: Probe::new(limit_ns),
        }
    }

    pub fn inner(&self) -> &N {
        &self.inner
    }
    pub fn inner_mut(&mut self) -> &mut N {
        &mut self.inner
    }
    pub fn into_inner(self) -> N {
        self.inner
    }
    pub fn probe(&self) -> &Probe {
        &self.probe
    }
    pub fn probe_mut(&mut self) -> &mut Probe {
        &mut self.probe
    }
    pub fn lifetime(&self) -> Summary {
        self.probe.lifetime()
    }
    pub fn take_window(&mut self) -> Summary {
        self.probe.take_window()
    }
}

impl<N: Node> Node for Profiled<N> {
    fn info(&self) -> &'static NodeInfo {
        self.inner.info()
    }
    fn params(&self) -> &'static [ParamInfo] {
        self.inner.params()
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.inner.set_param(id, value)
    }
    fn param(&self, id: u16) -> f32 {
        self.inner.param(id)
    }
    fn prepare(&mut self, rate: u32, max_block: usize) {
        self.inner.prepare(rate, max_block)
    }
    fn reset(&mut self) {
        self.inner.reset()
    }

    #[inline]
    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let span = Span::start();
        self.inner.process(ctx, events, left, right);
        self.probe.stop(span, left.len() as u32);
    }

    fn tail_frames(&self) -> u32 {
        self.inner.tail_frames()
    }
    fn latency(&self) -> u32 {
        self.inner.latency()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::{Kind, NodeEvent};

    static INFO: NodeInfo = NodeInfo {
        id: "test.gain",
        name: "Gain",
        kind: Kind::Effect,
    };

    struct Gain(f32, u32);

    impl Node for Gain {
        fn info(&self) -> &'static NodeInfo {
            &INFO
        }
        fn params(&self) -> &'static [ParamInfo] {
            &[]
        }
        fn set_param(&mut self, _: u16, v: f32) {
            self.0 = v;
        }
        fn param(&self, _: u16) -> f32 {
            self.0
        }
        fn prepare(&mut self, _: u32, _: usize) {}
        fn reset(&mut self) {
            self.1 += 1;
        }
        fn process(&mut self, _: &Ctx, ev: &[Timed], l: &mut [f32], r: &mut [f32]) {
            for e in ev {
                if let NodeEvent::Param { value, .. } = e.ev {
                    self.0 = value;
                }
            }
            for x in l.iter_mut().chain(r.iter_mut()) {
                *x *= self.0;
            }
        }
        fn latency(&self) -> u32 {
            7
        }
    }

    #[test]
    fn the_wrapper_is_transparent_and_counts_blocks() {
        let mut p = Profiled::new(Gain(0.5, 0), u64::MAX);
        let (mut l, mut r) = (vec![1.0f32; 64], vec![2.0f32; 64]);
        p.process(&Ctx::new(44_100, 0), &[], &mut l, &mut r);
        assert_eq!((l[0], r[63]), (0.5, 1.0));
        p.set_param(0, 2.0);
        assert_eq!(p.param(0), 2.0);
        p.process(&Ctx::new(44_100, 64), &[], &mut l[..32], &mut r[..32]);
        p.reset();
        assert_eq!((p.latency(), p.info().id, p.inner().1), (7, "test.gain", 1));
        let s = p.lifetime();
        assert_eq!((s.blocks, s.frames, s.over), (2, 96, 0));
        assert_eq!(p.take_window().blocks, 2);
        assert_eq!(p.take_window().blocks, 0);
        // Usable as a trait object, as in a chain slot.
        let _boxed: Box<dyn Node> = Box::new(p);
    }
}
