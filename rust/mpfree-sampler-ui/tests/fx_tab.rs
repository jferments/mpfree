//! The FX page's pedals name their knobs short enough for three abreast.

use mpfree_sampler_ui::fx_tab::knob_label;

#[test]
fn knob_names_shorten_for_the_pedals() {
    for (long, short) in [
        ("Cutoff", "Cutoff"),
        ("Resonance", "Reso"),
        ("Threshold", "Thresh"),
        ("Band 1 freq", "B1 freq"),
        ("Band 4 gain", "B4 gain"),
        ("Band 2 Q", "B2 Q"),
        ("Feedback", "Feedback"),
        ("Low frequency", "Low freq"),
    ] {
        assert_eq!(knob_label(long), short, "{long}");
    }
}
