//! Sharing the grain budget out by musical priority (the sampler plan,
//! phase 2 step 9).
//!
//! Before, the [`crate::engine::GRAIN_BUDGET`] went first come, first
//! served: the oldest clouds kept their grains and a new pad got what was
//! left. Now, before each part of a period, the clouds are ranked by what
//! the engine can see, and the budget is shared out rank by rank:
//!
//! 1. **A new hit** (a cloud in its first [`NEW_HIT_SECS`]): what it wants,
//!    unshared, out of everything.
//! 2. **Held pads and the master Grains FX:** each first a protected share
//!    of [`PROTECTED`] grains (or what it wants, if less), the youngest
//!    first; then what is left, evenly, up to what each wants.
//! 3. **Pads let go and ringing out, and clouds that have played longer
//!    than [`OLD_SECS`]:** what is left after that, evenly. They thin
//!    first, so a fresh snare outranks a texture that has played for 20
//!    seconds.
//!
//! Shares are even within a rank, not across ranks. While every cloud fits
//! with [`RESERVE`] grains to spare, nobody is shared at all, and the
//! clouds play exactly as before.
//!
//! A share only slows a cloud's new grains ([`GrainVoice::set_share`]): the
//! grains playing finish. So [`RESERVE`] grains are kept free for new
//! hits, and when fewer are free (or about to be), the oldest grains of the
//! lowest-ranked clouds over their shares fade out over [`FADE_SECS`],
//! along their own windows, to make room: never cut. When more clouds sound
//! than even the protected shares cover (past 30 held clouds), the youngest
//! keep theirs, and the rest make no new grains and finish the ones they
//! have, counted ([`Shared::held_back`]).
//!
//! [`GrainVoice::set_share`]: mpfree_grain::GrainVoice::set_share

use mpfree_grain::MAX_GRAINS;

/// How long a cloud counts as a new hit (seconds).
pub const NEW_HIT_SECS: f64 = 0.05;
/// A held cloud that has played this long thins with the tails (seconds).
pub const OLD_SECS: f64 = 10.0;
/// Each held cloud's protected share, in grains: 32 of them fill the 256.
pub const PROTECTED: usize = 8;
/// Grains kept free for new hits.
pub const RESERVE: usize = 16;
/// How long a grain ended to make room fades out (seconds).
pub const FADE_SECS: f64 = 0.005;

/// A cloud's rank (the [module docs](self)), most important first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rank {
    NewHit,
    Held,
    Tail,
}

/// One cloud, as the sharing sees it.
#[derive(Clone, Copy, Debug)]
pub struct Claim {
    pub rank: Rank,
    /// How long it has played, in frames ([`mpfree_grain::GrainVoice::played`]).
    pub played: u64,
    /// What it would keep alive unshared ([`mpfree_grain::GrainVoice::wants`]).
    pub wants: usize,
    /// What it has alive now.
    pub alive: usize,
    /// Its share, set by [`share`]: [`MAX_GRAINS`] is no share at all.
    pub share: usize,
}

impl Claim {
    pub fn new(rank: Rank, played: u64, wants: usize, alive: usize) -> Claim {
        Claim {
            rank,
            played,
            wants: wants.min(MAX_GRAINS),
            alive,
            share: MAX_GRAINS,
        }
    }

    /// What it holds or asks for, whichever is more.
    fn demand(&self) -> usize {
        self.wants.max(self.alive)
    }
}

/// What [`share`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shared {
    /// Whether the budget was crowded, so shares were set.
    pub crowded: bool,
    /// Clouds that want grains and got none: they finish the ones they
    /// have.
    pub held_back: usize,
}

/// The rank of a pad's cloud that has played `played` frames at `rate`,
/// held or let go.
pub fn rank_of(played: u64, held: bool, rate: u32) -> Rank {
    let secs = played as f64 / f64::from(rate.max(1));
    if secs < NEW_HIT_SECS {
        Rank::NewHit
    } else if held && secs < OLD_SECS {
        Rank::Held
    } else {
        Rank::Tail
    }
}

/// Sets each claim's share of `total` grains (the [module docs](self)).
/// Real-time safe: no allocation, and a few passes over the claims.
pub fn share(claims: &mut [Claim], total: usize) -> Shared {
    let demand: usize = claims.iter().map(Claim::demand).sum();
    if demand + RESERVE <= total {
        for c in claims.iter_mut() {
            c.share = MAX_GRAINS;
        }
        return Shared::default();
    }
    let mut left = total.saturating_sub(RESERVE);
    // New hits: what they want, unshared.
    for c in claims.iter_mut() {
        if c.rank == Rank::NewHit {
            c.share = MAX_GRAINS;
            left = left.saturating_sub(c.demand());
        } else {
            c.share = 0;
        }
    }
    // Held clouds' protected shares, the youngest first, while they last.
    loop {
        let mut youngest: Option<usize> = None;
        for (i, c) in claims.iter().enumerate() {
            if c.rank == Rank::Held
                && c.share == 0
                && c.wants > 0
                && youngest.is_none_or(|y| c.played < claims[y].played)
            {
                youngest = Some(i);
            }
        }
        let Some(i) = youngest else { break };
        let give = PROTECTED.min(claims[i].wants);
        if give > left {
            break;
        }
        claims[i].share = give;
        left -= give;
    }
    fill(claims, Rank::Held, &mut left);
    fill(claims, Rank::Tail, &mut left);
    Shared {
        crowded: true,
        held_back: claims
            .iter()
            .filter(|c| c.rank != Rank::NewHit && c.wants > 0 && c.share == 0)
            .count(),
    }
}

/// Shares `left` out evenly among the clouds of `rank`, up to what each
/// wants.
fn fill(claims: &mut [Claim], rank: Rank, left: &mut usize) {
    loop {
        let open = claims
            .iter()
            .filter(|c| c.rank == rank && c.share < c.wants)
            .count();
        if open == 0 || *left == 0 {
            return;
        }
        let each = (*left / open).max(1);
        for c in claims.iter_mut() {
            if c.rank != rank || c.share >= c.wants {
                continue;
            }
            let give = each.min(c.wants - c.share).min(*left);
            c.share += give;
            *left -= give;
            if *left == 0 {
                return;
            }
        }
    }
}

/// How many grains to end early for room, and from which clouds: with
/// `free` grains free now and `ending` about to end, the shortfall from
/// [`RESERVE`], taken from the clouds over their shares, the lowest rank
/// and the oldest first, never from a new hit. Calls `fade(i, n)` for each
/// (claim `i`, `n` grains), which says how many it ended. Returns how many
/// were ended.
pub fn make_room(
    claims: &[Claim],
    free: usize,
    ending: usize,
    mut fade: impl FnMut(usize, usize) -> usize,
) -> usize {
    let mut need = RESERVE.saturating_sub(free + ending);
    let mut done = 0;
    for rank in [Rank::Tail, Rank::Held] {
        // Oldest first: each pass takes the oldest not yet asked.
        let mut asked = 0u128;
        while need > 0 {
            let mut oldest: Option<usize> = None;
            for (i, c) in claims.iter().enumerate() {
                if c.rank == rank
                    && c.alive > c.share
                    && (i >= 128 || asked & (1 << i) == 0)
                    && oldest.is_none_or(|o| c.played > claims[o].played)
                {
                    oldest = Some(i);
                }
            }
            let Some(i) = oldest else { break };
            if i < 128 {
                asked |= 1 << i;
            }
            let c = &claims[i];
            let n = fade(i, need.min(c.alive - c.share));
            need = need.saturating_sub(n);
            done += n;
            if i >= 128 {
                // Past what the mask tracks: ask it once and stop, so the
                // pass always ends.
                break;
            }
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(rank: Rank, played: u64, wants: usize, alive: usize) -> Claim {
        Claim::new(rank, played, wants, alive)
    }

    #[test]
    fn nobody_is_shared_while_everyone_fits_with_room_to_spare() {
        let mut c = [
            claim(Rank::Held, 100, 54, 50),
            claim(Rank::Tail, 9_000, 54, 54),
            claim(Rank::NewHit, 1, 20, 3),
        ];
        assert_eq!(share(&mut c, 256), Shared::default());
        assert!(c.iter().all(|c| c.share == MAX_GRAINS));
    }

    #[test]
    fn a_new_hit_first_then_held_clouds_protected_then_tails() {
        // Six held clouds and four tails that each want 54: far past 256.
        let mut c: Vec<Claim> = (0..6)
            .map(|k| claim(Rank::Held, 1_000 + k, 54, 40))
            .chain((0..4).map(|k| claim(Rank::Tail, 50_000 + k, 54, 40)))
            .chain([claim(Rank::NewHit, 10, 30, 0)])
            .collect();
        let s = share(&mut c, 256);
        assert!(s.crowded);
        assert_eq!(c[10].share, MAX_GRAINS, "the hit unshared");
        // 256 - 16 kept free - 30 for the hit = 210 for the rest: the six
        // held clouds get 35 each and the tails nothing.
        let held: Vec<usize> = c[..6].iter().map(|c| c.share).collect();
        assert_eq!(held, vec![35; 6]);
        assert!(c[6..10].iter().all(|c| c.share == 0));
        assert_eq!(s.held_back, 4);
        let total: usize = c[..10].iter().map(|c| c.share).sum::<usize>() + 30;
        assert!(total + RESERVE <= 256);
    }

    #[test]
    fn shares_are_even_within_a_rank_and_tails_get_what_is_left() {
        let mut c = [
            claim(Rank::Held, 10_000, 54, 54),
            claim(Rank::Held, 2_000, 20, 20),
            claim(Rank::Tail, 900_000, 54, 54),
            claim(Rank::Tail, 400_000, 54, 54),
            claim(Rank::Tail, 100_000, 54, 54),
        ];
        // 236 wanted, out of 240 with 16 kept free: crowded.
        assert!(share(&mut c, 240).crowded);
        // 224 to share: the held clouds take what they want (54 and 20),
        // and the three tails share the 150 left evenly.
        assert_eq!(c[0].share, 54);
        assert_eq!(c[1].share, 20);
        let tails: Vec<usize> = c[2..].iter().map(|c| c.share).collect();
        assert_eq!(tails, vec![50, 50, 50]);
        // Crowded harder: the tails share evenly, within one grain.
        let mut c: Vec<Claim> = (0..3)
            .map(|k| claim(Rank::Held, k, 54, 54))
            .chain((0..5).map(|k| claim(Rank::Tail, 100_000 + k, 54, 54)))
            .collect();
        share(&mut c, 256);
        assert!(c[..3].iter().all(|c| c.share == 54));
        let tails: Vec<usize> = c[3..].iter().map(|c| c.share).collect();
        let (lo, hi) = (*tails.iter().min().unwrap(), *tails.iter().max().unwrap());
        assert!(hi - lo <= 1, "{tails:?}");
        assert_eq!(tails.iter().sum::<usize>(), 240 - 3 * 54);
    }

    #[test]
    fn past_thirty_held_clouds_the_youngest_keep_their_protected_share() {
        // 40 held clouds: 240 / 8 = 30 protected shares; the 10 oldest get
        // none and finish the grains they have.
        let mut c: Vec<Claim> = (0..40)
            .map(|k| claim(Rank::Held, 1_000 * k, 54, 8))
            .collect();
        let s = share(&mut c, 256);
        assert_eq!(s.held_back, 10);
        assert!(c[..30].iter().all(|c| c.share == PROTECTED));
        assert!(c[30..].iter().all(|c| c.share == 0));
        // A cloud that wants fewer than the protected share gets what it
        // wants, and the one that wants more takes what is left.
        let mut c: Vec<Claim> = (0..40).map(|k| claim(Rank::Held, k, 5, 5)).collect();
        c.push(claim(Rank::Held, 99, 54, 54));
        assert!(share(&mut c, 256).crowded);
        assert!(c[..40].iter().all(|c| c.share == 5));
        assert_eq!(c[40].share, 240 - 40 * 5);
    }

    #[test]
    fn room_is_made_from_the_lowest_rank_and_the_oldest_over_their_share() {
        let mut c = [
            claim(Rank::Held, 5_000, 54, 60),
            claim(Rank::Tail, 90_000, 54, 50),
            claim(Rank::Tail, 70_000, 54, 50),
            claim(Rank::NewHit, 10, 40, 0),
        ];
        c[0].share = 40;
        c[1].share = 45;
        c[2].share = 40;
        // 2 free, 1 about to end: 13 short of the reserve.
        let mut asked = Vec::new();
        let done = make_room(&c, 2, 1, |i, n| {
            asked.push((i, n));
            n
        });
        assert_eq!(done, 13);
        // The oldest tail gives its 5 over, the next tail the 8 still
        // short; the held cloud and the hit nothing.
        assert_eq!(asked, vec![(1, 5), (2, 8)]);
        // Enough free or ending: nothing.
        assert_eq!(make_room(&c, 10, 6, |_, n| n), 0);
    }

    #[test]
    fn ranks_follow_how_new_a_hit_is_whether_held_and_how_long_it_played() {
        let rate = 48_000;
        assert_eq!(rank_of(1_000, true, rate), Rank::NewHit);
        assert_eq!(rank_of(1_000, false, rate), Rank::NewHit);
        assert_eq!(rank_of(48_000, true, rate), Rank::Held);
        assert_eq!(rank_of(48_000, false, rate), Rank::Tail);
        assert_eq!(rank_of(48_000 * 11, true, rate), Rank::Tail);
    }
}
