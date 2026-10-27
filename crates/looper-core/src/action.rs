use std::time::Instant;

use crate::config::AppConfig;

/// What a rendered frame asks the app to do next.
///
/// Lives down here rather than with the app because both sides need it:
/// the screens return it and the shell acts on it. Putting it with the
/// shell would have the screens depending on the thing that calls them.
pub enum Action {
    Start(AppConfig),
    OpenSettings,
    /// The settings form's values, changed: for `SettingsState::edit`.
    EditSettings(AppConfig),
    /// One of the looper screen's own controls, for `LooperState::act`.
    Looper(LooperAction),
}

/// The looper screen's controls, as asked for - the screen reads the
/// looper and never changes it. Space and S aren't here: the shell
/// reads them and hands them to `tick`, which times their holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LooperAction {
    /// The round button went down, or the pointer let go - latched for
    /// the next tick, the way Space is read.
    ButtonHeld(bool),
    /// The same, for the stop button.
    StopButtonHeld(bool),
    Undo,
    Redo,
    ToggleMetronome,
    /// Slower or faster by this many BPM.
    NudgeTempo(i32),
    HalveTempo,
    DoubleTempo,
    /// When the tap landed: on the way down, not when the screen got
    /// round to it.
    TapTempo(Instant),
    LoopVolume(u32),
    ClickVolume(u32),
    /// A level let go of: saved.
    SaveLevels,
    ToggleMode,
    ToggleLayerMute(usize),
}
