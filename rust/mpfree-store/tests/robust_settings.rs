//! Robustness: any settings file text (damaged, hand-edited, another
//! version's) reads without a panic, and what it reads writes back to text
//! that reads the same.

#[path = "../../test-support/mutate.rs"]
mod mutate;

use mpfree_store::Settings;
use mutate::Mutator;

#[test]
fn any_settings_text_reads_and_round_trips() {
    let mut s = Settings::default();
    s.set_str("theme", "Console");
    s.set_bool("show_developer_tools", true);
    s.set_i64("periods", 3);
    s.set_f64("touch.scale_x", 1.0125);
    let seed = format!("# MPFree settings\n\n{}", s.to_text());
    let mut m = Mutator::new(14);
    for case in 0..20_000 {
        let text = if case % 4 == 0 {
            let n = m.below(200);
            m.text("ab_.=# \n\t-0123456789", n)
        } else {
            m.mutate_text(&seed)
        };
        let read = Settings::parse(&text);
        let again = Settings::parse(&read.to_text());
        assert_eq!(again, read, "case {case}: {text:?}");
        for (k, _) in read.iter() {
            let _ = (read.get_bool(k), read.get_i64(k), read.get_f64(k));
        }
    }
}
