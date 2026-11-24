//! The note editor (the piano roll plan) through the whole session, as a
//! musician works it: opened from the Sequencer's Steps | Notes switch on
//! a clip drawn on the Steps, then a finger resting on a note while the
//! data wheel turns, ERASE held while a finger drags along a row, and the
//! clip's end dragged on the ruler. Every step goes through the screen,
//! the desk and the MPC's buttons, and is checked against the time
//! engine's clip, not only the screen, then taken back with UNDO.

#[cfg(test)]
mod tests {
    use super::super::{on_big_stack, Session};
    use mpfree_seq::Clip;
    use mpfree_time_ui::view::NOTES;
    use mpfree_ui::{Rect, Touch};

    /// The finger the tests touch with (any but the sweep's).
    const FINGER: u64 = 7;

    /// A session with four kicks drawn on the Steps (steps 1, 5, 9 and
    /// 13 of a one-bar clip), then the note editor opened on that clip by
    /// the switch's Notes.
    fn kicks_in_the_editor() -> Session {
        let mut s = Session::new();
        s.play_view("Beats");
        for k in [1, 5, 9, 13] {
            s.press("steps", "step", &format!("Kick step {k}"));
        }
        s.press(crate::play_page::TRANSPORT, "view", "Notes")
            .settle();
        assert!(s.time.editing(), "the switch's Notes opens the note editor");
        assert_eq!(clip(&s).notes.len(), 4);
        s
    }

    /// The clip the editor is on, as the time engine has it.
    fn clip(s: &Session) -> Clip {
        let (t, slot) = s.time.state().focus().expect("a clip in focus");
        s.time.project().clip(t, slot).expect("its clip").clone()
    }

    /// The velocities of the clip's notes in time order (an edit or an
    /// undo may put them back in another order, which plays the same).
    fn velocities(s: &Session) -> Vec<u8> {
        let mut n = clip(s).notes;
        n.sort_by_key(|n| (n.tick, n.note));
        n.iter().map(|n| n.velocity).collect()
    }

    /// The roll's notes as drawn now, in time order.
    fn notes_on_the_roll(s: &mut Session) -> Vec<Rect> {
        let mut r: Vec<Rect> = s
            .controls()
            .into_iter()
            .filter(|c| c.widget == NOTES && c.target.id == "note")
            .map(|c| c.target.rect)
            .collect();
        r.sort_by_key(|r| r.x);
        r
    }

    /// The middle of `r`.
    fn mid(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    /// A finger put down, moved or lifted at `(x, y)`, everything the
    /// screen sends for it routed as the session routes a press.
    fn touch(s: &mut Session, phase: Touch, x: i32, y: i32) {
        let first = s.screen.touch(FINGER, phase, x, y);
        let rest = s.screen.take_actions();
        for a in first.into_iter().chain(rest) {
            s.route(a);
        }
        s.run_periods(4);
    }

    /// A finger on a note puts its velocity under the data wheel (piano
    /// roll plan 1C.3): three clicks back make that kick 3 softer and no
    /// other, the hint line says so, the lift taps nothing, and one UNDO
    /// takes the whole hold back.
    #[test]
    fn a_finger_on_a_note_and_the_wheel_turn_its_velocity_one_undo() {
        on_big_stack(|| {
            let mut s = kicks_in_the_editor();
            let before = velocities(&s);
            let second = notes_on_the_roll(&mut s)[1];
            let (x, y) = mid(second);
            touch(&mut s, Touch::Down, x, y);
            s.wheel(-1).wheel(-1).wheel(-1);
            let mut want = before.clone();
            want[1] -= 3;
            assert_eq!(velocities(&s), want, "the second kick, 3 softer");
            assert!(
                s.hint().starts_with(&format!("Velocity {}", want[1])),
                "{}",
                s.hint()
            );
            touch(&mut s, Touch::Up, x, y);
            s.settle();
            assert_eq!(velocities(&s), want, "the lift taps nothing");
            assert_eq!(clip(&s).notes.len(), 4);
            // The wheel is the page's again once the finger is off.
            s.wheel(-1);
            assert_eq!(velocities(&s), want);
            s.button("UNDO").settle();
            assert_eq!(velocities(&s), before, "one UNDO takes the hold back");
        });
    }

    /// With the kicks picked, a finger on one of them turns them all as
    /// Q4 does, each keeping its difference, and SHIFT and the wheel widen
    /// the gap between the soft one and the loud ones (piano roll plan
    /// 1C.3); the hint line says what turns, and one UNDO a hold takes it
    /// back.
    #[test]
    fn a_finger_on_a_picked_note_turns_the_picks_and_shift_widens_the_gap() {
        on_big_stack(|| {
            let mut s = kicks_in_the_editor();
            let v = velocities(&s)[0];
            assert_eq!(velocities(&s), [v; 4]);
            // The first kick alone 8 softer.
            let roll = notes_on_the_roll(&mut s);
            let (x, y) = mid(roll[0]);
            touch(&mut s, Touch::Down, x, y);
            s.wheel(-8);
            touch(&mut s, Touch::Up, x, y);
            assert_eq!(velocities(&s), [v - 8, v, v, v]);
            // Select › All, then a finger on the second kick.
            s.press(NOTES, "button", "Select");
            s.press(NOTES, "select", "All").settle();
            let roll = notes_on_the_roll(&mut s);
            let (x, y) = mid(roll[1]);
            touch(&mut s, Touch::Down, x, y);
            s.wheel(1).wheel(1);
            assert_eq!(velocities(&s), [v - 6, v + 2, v + 2, v + 2]);
            let said = format!("4 notes' velocity {} to {}", v - 6, v + 2);
            assert!(s.hint().starts_with(&said), "{}", s.hint());
            // SHIFT: the gap of 8 a step wider at each end.
            s.wheel_with("SHIFT", 1);
            assert_eq!(velocities(&s), [v - 7, v + 3, v + 3, v + 3]);
            touch(&mut s, Touch::Up, x, y);
            s.settle();
            assert_eq!(clip(&s).notes.len(), 4);
            s.button("UNDO").settle();
            assert_eq!(velocities(&s), [v - 8, v, v, v], "UNDO: the whole hold");
            s.button("UNDO").settle();
            assert_eq!(velocities(&s), [v; 4], "UNDO: the first hold");
        });
    }

    /// SHIFT + the wheel's push with a finger on a note puts its velocity
    /// back to 100, the velocity a drawn note gets (Jay's card, 23 Nov
    /// 2026), the hint line says so, and it joins the hold's take: one
    /// UNDO takes back the turn and the reset together.
    #[test]
    fn shift_and_the_push_on_a_held_note_put_its_velocity_back_to_100() {
        on_big_stack(|| {
            let mut s = kicks_in_the_editor();
            let v = velocities(&s)[1];
            let (x, y) = mid(notes_on_the_roll(&mut s)[1]);
            // One hold: the second kick 8 softer than 100 and lifted.
            touch(&mut s, Touch::Down, x, y);
            s.wheel(100 - i32::from(v) - 8);
            touch(&mut s, Touch::Up, x, y);
            s.settle();
            assert_eq!(velocities(&s)[1], 92);
            // The next: turned on, then SHIFT + the push.
            touch(&mut s, Touch::Down, x, y);
            s.wheel(-3);
            assert_eq!(velocities(&s)[1], 89);
            s.button_with("SHIFT", "DATA DIAL PUSH");
            assert_eq!(velocities(&s)[1], 100, "back to a drawn note's velocity");
            assert!(s.hint().starts_with("Velocity 100"), "{}", s.hint());
            assert!(s.hint().contains("back to its default"), "{}", s.hint());
            touch(&mut s, Touch::Up, x, y);
            s.settle();
            assert_eq!(clip(&s).notes.len(), 4, "the lift taps nothing");
            s.button("UNDO").settle();
            assert_eq!(velocities(&s)[1], 92, "one UNDO: the turn and the reset");
        });
    }

    /// ERASE held, a finger dragged along the kicks' row takes out every
    /// note it crosses, as one edit on the lift (piano roll plan 1B.5),
    /// and one UNDO brings them all back.
    #[test]
    fn erase_held_and_a_drag_along_a_row_take_its_notes_out_one_undo() {
        on_big_stack(|| {
            let mut s = kicks_in_the_editor();
            let roll = notes_on_the_roll(&mut s);
            let (from, to) = (roll[0], roll[2]);
            let y = mid(from).1;
            s.button_edge("ERASE", true);
            touch(&mut s, Touch::Down, from.x + from.w / 2, y);
            let steps = 8;
            for k in 1..=steps {
                let x = from.x + from.w / 2 + (mid(to).0 - mid(from).0) * k / steps;
                touch(&mut s, Touch::Move, x, y);
            }
            assert_eq!(clip(&s).notes.len(), 4, "nothing goes until the lift");
            touch(&mut s, Touch::Up, mid(to).0, y);
            s.button_edge("ERASE", false).settle();
            assert_eq!(clip(&s).notes.len(), 1, "the first three kicks are out");
            s.button("UNDO").settle();
            assert_eq!(clip(&s).notes.len(), 4, "one UNDO brings them back");
        });
    }

    /// The clip's end on the ruler (piano roll plan 1A.2): on the
    /// one-bar clip of four kicks the grip dragged a step in makes it 15
    /// steps; dragged on past the last kick it stops a step after it (13
    /// steps), every kick kept; each lift is one take that UNDO takes back.
    #[test]
    fn the_clips_end_dragged_in_shortens_it_to_the_last_kick_one_undo_a_drag() {
        on_big_stack(|| {
            let mut s = kicks_in_the_editor();
            let bar = clip(&s).length;
            let step = bar / 16;
            let grip = s.control(NOTES, "end", "").expect("the end's grip").target;
            s.press_target(&grip).settle();
            assert_eq!(clip(&s).length, 15 * step, "a step in: 15 steps");
            // On past the last kick (step 13), to the roll's first note.
            let grip = s.control(NOTES, "end", "").expect("the end's grip").target;
            let first = notes_on_the_roll(&mut s)[0];
            let (x, y) = mid(grip.rect);
            touch(&mut s, Touch::Down, x, y);
            for k in 1..=8 {
                touch(&mut s, Touch::Move, x + (first.x - x) * k / 8, y);
            }
            touch(&mut s, Touch::Up, first.x, y);
            s.settle();
            assert_eq!(
                clip(&s).length,
                13 * step,
                "it stops a step past the last kick"
            );
            assert_eq!(clip(&s).notes.len(), 4, "every kick kept");
            s.button("UNDO").settle();
            assert_eq!(clip(&s).length, 15 * step);
            s.button("UNDO").settle();
            assert_eq!(clip(&s).length, bar, "two UNDOs: the bar again");
        });
    }
}
