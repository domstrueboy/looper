use std::time::{Duration, Instant};

/// Turns "is it down right now" into a press and a long press, like a
/// real footswitch: the press fires the moment the control goes down, and
/// holding it on past the threshold fires the long press as well, while
/// still held.
///
/// On the way down, not the way up. A press that waited for the release,
/// to be sure it wasn't the start of a hold, started and stopped every
/// take as late as the key or foot stayed down: 100-200 ms of it. Acting
/// at once costs nothing, because whoever acts on a long press takes back
/// whatever the press began.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    None,
    Press,
    /// A second press within `DOUBLE_PRESS_WINDOW` of the first. The first
    /// has already been acted on by then - it fired as it went down - so
    /// whoever acts on this one decides what of the first to take back.
    DoublePress,
    LongPress,
}

/// How soon a second press has to follow the first to count as a double:
/// quick enough that two deliberate single presses don't run together.
pub const DOUBLE_PRESS_WINDOW: Duration = Duration::from_millis(400);

pub struct InputHandler {
    long_press_threshold: Duration,
    pressed_at: Option<Instant>,
    long_press_fired: bool,
    /// The last single press, while a second could still make it a
    /// double. Cleared by a double, so three quick presses are a double
    /// and a single rather than two doubles.
    last_press: Option<Instant>,
}

impl InputHandler {
    pub fn new(long_press_threshold: Duration) -> Self {
        Self {
            long_press_threshold,
            pressed_at: None,
            long_press_fired: false,
            last_press: None,
        }
    }

    /// The configured hold time, so the UI can say what it actually is
    /// rather than naming a fixed one.
    pub fn long_press_threshold(&self) -> Duration {
        self.long_press_threshold
    }

    /// True from the frame `LongPress` fired until release - for UI
    /// confirming it, distinct from the one-shot event itself.
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
                    match self.last_press.take() {
                        Some(last) if now.duration_since(last) <= DOUBLE_PRESS_WINDOW => {
                            InputEvent::DoublePress
                        }
                        _ => {
                            self.last_press = Some(now);
                            InputEvent::Press
                        }
                    }
                }
                Some(pressed_at) => {
                    if !self.long_press_fired
                        && now.duration_since(pressed_at) >= self.long_press_threshold
                    {
                        self.long_press_fired = true;
                        InputEvent::LongPress
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
