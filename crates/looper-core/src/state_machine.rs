/// The looper's states and the moves between them: the cycle Idle ->
/// Recording -> Looping -> Stopped -> Looping, a dub off Looping and
/// back, a wait before recording, and a clear from anywhere. Which
/// control makes which move, in which record mode, is
/// `looper/pedal.rs`'s business.
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
    /// Waiting to record: a delay counting down, a count-in clicking, or
    /// listening for the first note. Nothing is kept yet, but the
    /// recorder is already fed - a count-in's take opens on the bar line,
    /// a detected one with the attack before it, both on the audio side.
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

    /// Advances the cycle: records, ends a take into playing, stops
    /// either playing state, plays a stopped loop, calls a wait off.
    pub fn press(&mut self) {
        self.state = match self.state {
            LoopState::Idle => LoopState::Recording,
            LoopState::Recording => LoopState::Looping,
            LoopState::Looping | LoopState::Overdubbing => LoopState::Stopped,
            LoopState::Stopped => LoopState::Looping,
            LoopState::Arming => LoopState::Idle,
        };
    }

    /// Waits before recording rather than starting at once. Whether to
    /// is the caller's call - it's the one that knows the record start.
    pub fn arm(&mut self) {
        if self.state == LoopState::Idle {
            self.state = LoopState::Arming;
        }
    }

    /// The wait is over - the delay run out, the bar line reached, the
    /// first note heard: recording starts for real.
    pub fn finish_arming(&mut self) {
        if self.state == LoopState::Arming {
            self.state = LoopState::Recording;
        }
    }

    /// Opens a dub over the playing loop, or closes the one in progress.
    /// Nothing to do from any other state - there's no loop to dub onto.
    pub fn toggle_overdub(&mut self) {
        self.state = match self.state {
            LoopState::Looping => LoopState::Overdubbing,
            LoopState::Overdubbing => LoopState::Looping,
            other => other,
        };
    }

    /// Back to Idle from any state: holding S, or a hold or double press
    /// taking back a first take.
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
