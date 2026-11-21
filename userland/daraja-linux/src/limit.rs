//! A limit on a repeated console line (the architect's pre-L1 item 2, COMPAT_STAGE0_PLAN 18): a program that calls an
//! unknown number in a loop must not flood the console. The first `SHOWN` lines of each number are printed; the rest
//! are only counted, and one summary line per number says how many were not shown, when the program ends.
//!
//! Fixed size, no allocation, kept on the bridge's stack (the bridge has no writable statics): `SLOTS` numbers are
//! tracked one by one; calls of any further number share one "other" count, with its own `SHOWN` lines.

/// Lines printed per number (and for the shared "other" count) before the rest are only counted.
pub const SHOWN: u32 = 3;
/// Distinct numbers tracked one by one.
pub const SLOTS: usize = 16;

/// Whether to print this occurrence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Say {
    Show,
    Quiet,
}

/// The counts of one run.
pub struct Limit {
    nr: [u64; SLOTS],
    seen: [u32; SLOTS],
    used: usize,
    other: u32,
}

impl Limit {
    pub const fn new() -> Self {
        Limit { nr: [0; SLOTS], seen: [0; SLOTS], used: 0, other: 0 }
    }

    /// Count one occurrence of `nr`; Show for the first SHOWN of it, Quiet after.
    pub fn note(&mut self, nr: u64) -> Say {
        let seen = match self.nr[..self.used].iter().position(|&n| n == nr) {
            Some(i) => &mut self.seen[i],
            None if self.used < SLOTS => {
                self.nr[self.used] = nr;
                self.used += 1;
                &mut self.seen[self.used - 1]
            }
            None => &mut self.other,
        };
        *seen = seen.saturating_add(1);
        if *seen <= SHOWN {
            Say::Show
        } else {
            Say::Quiet
        }
    }

    /// The lines not shown: (Some(nr), count) per tracked number, then (None, count) for the shared other count;
    /// only entries with a count above zero. Each entry is given once: `take` clears it.
    pub fn take(&mut self) -> Option<(Option<u64>, u32)> {
        for i in 0..self.used {
            if self.seen[i] > SHOWN {
                let hidden = self.seen[i] - SHOWN;
                self.seen[i] = SHOWN;
                return Some((Some(self.nr[i]), hidden));
            }
        }
        if self.other > SHOWN {
            let hidden = self.other - SHOWN;
            self.other = SHOWN;
            return Some((None, hidden));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loop_of_one_number_shows_three_and_counts_the_rest() {
        let mut l = Limit::new();
        let shown = (0..10_000).filter(|_| l.note(999) == Say::Show).count();
        assert_eq!(shown, SHOWN as usize);
        assert_eq!(l.take(), Some((Some(999), 10_000 - SHOWN)));
        assert_eq!(l.take(), None); // given once
        // after a summary, the number stays quiet (no new burst of lines)
        assert_eq!(l.note(999), Say::Quiet);
        assert_eq!(l.take(), Some((Some(999), 1)));
    }

    #[test]
    fn a_few_calls_show_all_and_leave_no_summary() {
        let mut l = Limit::new();
        for nr in [1, 2, 2, 3, 3, 3] {
            assert_eq!(l.note(nr), Say::Show);
        }
        assert_eq!(l.take(), None);
    }

    #[test]
    fn many_numbers_share_the_other_count_and_stay_bounded() {
        let mut l = Limit::new();
        let mut shown = 0;
        // every number from 1000 up, five times each: SLOTS tracked, the rest in "other"
        for round in 0..5 {
            for nr in 1000..1100u64 {
                if l.note(nr) == Say::Show {
                    shown += 1;
                }
                let _ = round;
            }
        }
        // tracked: SHOWN each; other: SHOWN in all
        assert_eq!(shown, SLOTS * SHOWN as usize + SHOWN as usize);
        let mut lines = 0;
        let mut hidden = 0u32;
        while let Some((nr, n)) = l.take() {
            lines += 1;
            hidden += n;
            if nr.is_none() {
                assert_eq!(n, (100 - SLOTS as u32) * 5 - SHOWN);
            }
        }
        assert_eq!(lines, SLOTS + 1); // one per tracked number and one for the rest: bounded whatever the program does
        assert_eq!(hidden as usize + shown, 500);
    }

    #[test]
    fn the_count_saturates() {
        let mut l = Limit::new();
        l.seen[0] = u32::MAX - 1;
        l.nr[0] = 7;
        l.used = 1;
        assert_eq!(l.note(7), Say::Quiet);
        assert_eq!(l.note(7), Say::Quiet);
        assert_eq!(l.take(), Some((Some(7), u32::MAX - SHOWN)));
    }
}
