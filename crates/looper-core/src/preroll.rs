use std::time::{Duration, Instant};

/// The wait before the first recording starts, counted down by the UI
/// frame loop.
///
/// Pure time arithmetic, kept apart from the rest of the looper state
/// because nothing here touches the device - which is what lets it be
/// tested against a made-up `Instant` rather than a real stream.
pub struct Preroll {
    /// How long to wait. Zero means don't: recording starts the moment
    /// it's asked for.
    duration: Duration,
    /// Set while a countdown is running, cleared the moment it elapses
    /// or is called off.
    started_at: Option<Instant>,
}

impl Preroll {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            started_at: None,
        }
    }

    /// Whether there's a wait worth counting down at all.
    pub fn is_enabled(&self) -> bool {
        !self.duration.is_zero()
    }

    pub fn start(&mut self, now: Instant) {
        self.started_at = Some(now);
    }

    pub fn cancel(&mut self) {
        self.started_at = None;
    }

    /// True on the frame the wait runs out, and only that frame - it
    /// clears itself. False whenever nothing is counting down, so a
    /// countdown called off before then never fires.
    pub fn take_if_elapsed(&mut self, now: Instant) -> bool {
        match self.started_at {
            Some(started_at) if now.duration_since(started_at) >= self.duration => {
                self.started_at = None;
                true
            }
            _ => false,
        }
    }

    /// Seconds left, or 0.0 when nothing is counting down.
    pub fn remaining_secs(&self, now: Instant) -> f32 {
        match self.started_at {
            Some(started_at) => (self.duration.as_secs_f32()
                - now.duration_since(started_at).as_secs_f32())
            .max(0.0),
            None => 0.0,
        }
    }

    /// How far through the wait the countdown has got, 0.0-1.0, or 0.0
    /// when nothing is counting down.
    pub fn progress(&self, now: Instant) -> f32 {
        match self.started_at {
            Some(started_at) if self.is_enabled() => (now.duration_since(started_at).as_secs_f32()
                / self.duration.as_secs_f32())
            .clamp(0.0, 1.0),
            _ => 0.0,
        }
    }
}

#[cfg(test)]
#[path = "preroll_tests.rs"]
mod tests;
