use std::time::{Duration, Instant};

/// A tempo from taps: the average of the last few gaps between them.
///
/// Pure time arithmetic, like `Preroll`, so it's tested against made-up
/// `Instant`s. Taps come from UI frames, a frame apart at worst - the
/// averaging is what evens that out.
pub struct TapTempo {
    last: Option<Instant>,
    gaps: [Duration; TAP_GAPS],
    /// How many of `gaps` are real, up to `TAP_GAPS`.
    count: usize,
    /// Where the next gap goes, round and round.
    next: usize,
}

/// Gaps averaged: enough to steady a hand, few enough to follow a change
/// of mind within a bar.
pub const TAP_GAPS: usize = 4;

/// A pause this long starts a fresh count - longer than any gap the
/// slowest tempo (40 BPM, 1.5 s) needs.
pub const TAP_TIMEOUT: Duration = Duration::from_secs(2);

impl TapTempo {
    pub fn new() -> Self {
        Self {
            last: None,
            gaps: [Duration::ZERO; TAP_GAPS],
            count: 0,
            next: 0,
        }
    }

    /// A tap at `now`: the tempo so far in BPM, or `None` for the first
    /// tap of a count.
    pub fn tap(&mut self, now: Instant) -> Option<f64> {
        match self.last {
            Some(last) if now.saturating_duration_since(last) <= TAP_TIMEOUT => {
                self.gaps[self.next] = now - last;
                self.next = (self.next + 1) % TAP_GAPS;
                self.count = (self.count + 1).min(TAP_GAPS);
            }
            _ => {
                self.count = 0;
                self.next = 0;
            }
        }
        self.last = Some(now);
        if self.count == 0 {
            return None;
        }
        let total: Duration = self.gaps[..self.count].iter().sum();
        Some(60.0 * self.count as f64 / total.as_secs_f64())
    }
}

impl Default for TapTempo {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "tap_tempo_tests.rs"]
mod tests;
