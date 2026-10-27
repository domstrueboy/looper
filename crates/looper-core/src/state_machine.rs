/// Mimics a classic single-footswitch looper pedal (e.g. TC Electronic
/// Ditto): one control cycles Idle -> Recording -> Looping -> Stopped ->
/// Looping -> ..., and a long-press clears from any state back to Idle.
/// Overdub sits off that cycle on a control of its own, so the press
/// cycle stays the four states above and nothing else.
///
/// The discriminants cross the thread boundary as a `u8` - see
/// `SharedControl::load_state` - so their order is not free to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopState {
    Idle,
    Recording,
    Looping,
    Stopped,
    Overdubbing,
    /// Counting down a configured pre-roll before recording starts, so
    /// there's time to get hands back on the guitar. The audio thread
    /// treats this like Idle - nothing is captured yet - but it's a
    /// published state of its own because a count-in eventually needs to
    /// click here.
    Arming,
}

pub struct LoopStateMachine {
    state: LoopState,
}

impl LoopStateMachine {
    pub fn new() -> Self {
        Self {
            state: LoopState::Idle,
        }
    }

    /// A machine for a loop that was restored at startup: it exists, but
    /// nothing plays until asked.
    pub fn stopped() -> Self {
        Self {
            state: LoopState::Stopped,
        }
    }

    pub fn state(&self) -> LoopState {
        self.state
    }

    /// Short press: advances the cycle. Both playing states stop, so the
    /// main control always means "stop" while something is playing, and a
    /// press part-way through a pre-roll calls it off.
    pub fn press(&mut self) {
        self.state = match self.state {
            LoopState::Idle => LoopState::Recording,
            LoopState::Recording => LoopState::Looping,
            LoopState::Looping | LoopState::Overdubbing => LoopState::Stopped,
            LoopState::Stopped => LoopState::Looping,
            LoopState::Arming => LoopState::Idle,
        };
    }

    /// Starts a pre-roll instead of recording straight away. Whether a
    /// press comes here or to `press` is the caller's call - it's the one
    /// that knows whether a delay is configured.
    pub fn arm(&mut self) {
        if self.state == LoopState::Idle {
            self.state = LoopState::Arming;
        }
    }

    /// The pre-roll elapsed: recording starts for real.
    pub fn finish_arming(&mut self) {
        if self.state == LoopState::Arming {
            self.state = LoopState::Recording;
        }
    }

    /// The overdub control: opens a new layer over the playing loop, or
    /// closes the one in progress. Nothing to do from any other state -
    /// there's no loop to overdub onto.
    pub fn toggle_overdub(&mut self) {
        self.state = match self.state {
            LoopState::Looping => LoopState::Overdubbing,
            LoopState::Overdubbing => LoopState::Looping,
            other => other,
        };
    }

    /// Long-press: clears the loop from any state, back to Idle. How long
    /// a hold that takes is `long_press_ms`, and `InputHandler`'s to
    /// measure.
    pub fn clear(&mut self) {
        self.state = LoopState::Idle;
    }
}

impl Default for LoopStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "state_machine_tests.rs"]
mod tests;
