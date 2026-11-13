//! Workflows: what a musician does, step by step, through the whole
//! session (the screen and the control side together), each step checked.

#[cfg(test)]
mod tests {
    use super::super::{on_big_stack, walk};

    #[test]
    fn explore() {
        on_big_stack(|| {
            let (mut s, trail) = walk::replay(16, 56);
            let dump = |s: &mut super::super::Session| {
                let st = s.time.steps();
                println!(
                    "page {} track {} clip {:?}",
                    s.page(),
                    st.track,
                    st.clip.as_ref().map(|c| (c.name.clone(), c.steps))
                );
                if let Some(c) = &st.clip {
                    for (p, r) in &c.rows {
                        let on: Vec<usize> = r
                            .iter()
                            .enumerate()
                            .filter(|(_, x)| x.is_some())
                            .map(|(i, _)| i + 1)
                            .collect();
                        if !on.is_empty() {
                            println!("  clip row {p}: {on:?}");
                        }
                    }
                }
                if let Some(g) = s
                    .screen
                    .ui
                    .get::<crate::play_widgets::StepGrid>(crate::play_page::STEPS)
                {
                    println!("  grid '{}' '{}' steps {}", g.title, g.right, g.steps);
                    for l in &g.lanes {
                        let on: Vec<usize> = l
                            .steps
                            .iter()
                            .enumerate()
                            .filter(|(_, &v)| v > 0)
                            .map(|(i, _)| i + 1)
                            .collect();
                        if !on.is_empty() {
                            println!("  lane {} {}: {on:?}", l.pad, l.name);
                        }
                    }
                }
                println!("  transport {:?}", s.time.info().state);
            };
            s.press("steps", "step", "F3 step 8");
            for k in 0..400 {
                let before = s.time.steps().clip.map(|c| {
                    c.row(0)
                        .iter()
                        .enumerate()
                        .filter(|(_, x)| x.is_some())
                        .map(|(i, _)| i + 1)
                        .collect::<Vec<_>>()
                });
                let ch = s.time.take_steps_changed();
                if ch {
                    let st = s.time.steps();
                    s.screen.apply(crate::gui::ToUi::Steps(Box::new(st)));
                }
                s.run_periods(1);
                let g = s
                    .screen
                    .ui
                    .get::<crate::play_widgets::StepGrid>(crate::play_page::STEPS)
                    .map(|g| {
                        g.lanes.iter().find(|l| l.pad == 0).map(|l| {
                            l.steps
                                .iter()
                                .enumerate()
                                .filter(|(_, &v)| v > 0)
                                .map(|(i, _)| i + 1)
                                .collect::<Vec<_>>()
                        })
                    });
                if k % 40 == 0 || ch {
                    println!(
                        "{k}: changed {ch} clip row0 {before:?} grid {g:?} pos {:?}",
                        s.time.info().position
                    );
                }
            }
        });
    }
}
