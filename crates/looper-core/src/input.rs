use std::time::{Duration, Instant};

/// Turns "is it down right now" into a press and a long-press-clear, like
/// a real footswitch: the press fires the moment the control goes down,
/// and holding it on past the threshold clears as well, while still held.
///
/// On the way down, not the way up. A press that waited for the release,
/// to be sure it wasn't the start of a hold, started and stopped every
/// take as late as the key or foot stayed down: 100-200 ms of it. Acting
/// at once costs nothing, because whatever the press began is wiped by
/// the clear a hold turns into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    None,
    Press,
    LongPressClear,
}

pub struct InputHandler {
    long_press_threshold: Duration,
    pressed_at: Option<Instant>,
    long_press_fired: bool,
}

impl InputHandler {
    pub fn new(long_press_threshold: Duration) -> Self {
        Self {
            long_press_threshold,
            pressed_at: None,
            long_press_fired: false,
        }
    }

    /// The configured hold time, so the UI can say what it actually is
    /// rather than naming a fixed one.
    pub fn long_press_threshold(&self) -> Duration {
        self.long_press_threshold
    }

    /// True from the frame `LongPressClear` fired until release - for UI
    /// confirming the clear, distinct from the one-shot event itself.
    pub fn is_long_press_active(&self) -> bool {
        self.pressed_at.is_some() && self.long_press_fired
    }

    /// Call once per frame.
    pub fn update(&mut self, is_down: bool, now: Instant) -> InputEvent {
        if is_down {
            match self.pressed_at {
                None => {
                    self.pressed_at = Some(now);
                    self.long_press_fired = false;
                    InputEvent::Press
                }
                Some(pressed_at) => {
                    if !self.long_press_fired
                        && now.duration_since(pressed_at) >= self.long_press_threshold
                    {
                        self.long_press_fired = true;
                        InputEvent::LongPressClear
                    } else {
                        InputEvent::None
                    }
                }
            }
        } else {
            self.pressed_at = None;
            InputEvent::None
        }
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
