use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use looper_hal::{AudioStream, Backends};

use crate::audio::engine;
use crate::audio::fault::StreamFault;
use crate::audio::shared_control::SharedControl;
use crate::state_machine::{LoopState, LoopStateMachine};
use crate::config::{self, AppConfig, Mode};
use crate::input::{InputEvent, InputHandler};
use crate::loop_mirror::{self, LoopMirror};
use crate::preroll::Preroll;

/// State behind the looper screen: the state machine (owned here, on the
/// UI thread), the relay into the audio thread, and the live streams.
pub struct LooperState {
    control: Arc<SharedControl>,
    state_machine: LoopStateMachine,
    input_handler: InputHandler,
    // Latched on press, cleared only when the pointer is released
    // anywhere: egui reads a cursor drifting off the button as a release
    // even while it's still held, which broke long-press-clear from the
    // on-screen button. Set by the renderer, kept here to survive frames.
    button_held: bool,
    /// The countdown before the first recording starts.
    preroll: Preroll,
    /// Kept here as well as in the layer stack, which lives on the audio
    /// thread out of reach - the UI needs it to show "n/max" and to know
    /// when overdubbing is still possible.
    max_layers: usize,
    /// Which layers are muted, one bit each - owned here and published
    /// to the audio thread whole. Not saved: a loop reopens with every
    /// layer playing.
    muted_layers: u32,
    /// The loop as the UI thread sees it, which is the only copy that can
    /// be written to disk - see `LoopMirror`.
    mirror: LoopMirror,
    sample_rate: u32,
    /// The settings this looper was started with. Kept so a change made
    /// from this screen - the mode - can be saved without losing the rest.
    config: AppConfig,
    /// Where the loop and the config are written. Held rather than looked
    /// up each time so a test can point it somewhere harmless -
    /// `config::app_dir()` is the real per-user one.
    dir: PathBuf,
    /// When the current frame was ticked. The readouts below answer from
    /// this rather than asking the clock again, so what a frame shows
    /// can't disagree with the state it was drawn beside.
    now: Instant,
    /// Where the stream leaves word if it stops on its own - a device
    /// unplugged mid-loop, or a driver giving up.
    fault: StreamFault,
    /// Underruns since this looper started. Accumulated rather than
    /// shown as they happen: they arrive in bursts, and a line that
    /// flickers is one nobody reads.
    underruns: usize,
    /// Holding it is what keeps the audio running; dropping it stops.
    _stream: Box<dyn AudioStream>,
}

impl LooperState {
    /// Opens the device and starts the streams, or returns why it couldn't:
    /// device/rate mismatches are user-recoverable, not bugs.
    pub fn start(backends: &Backends, settings: &AppConfig) -> Result<Self, String> {
        Self::start_in(backends, settings, config::app_dir())
    }

    /// The same, with the saved loop and config somewhere other than the
    /// per-user directory. Split out for the tests, which must not read or
    /// write the files of whoever runs them.
    pub fn start_in(
        backends: &Backends,
        settings: &AppConfig,
        dir: PathBuf,
    ) -> Result<Self, String> {
        let control = Arc::new(SharedControl::new(settings.volume_pct));
        // Whatever was left from last time, if it still fits what's
        // configured now.
        let restored = loop_mirror::load(&config::loop_dir_in(&dir), settings);
        let fault = StreamFault::default();
        let streams = engine::build_looper_streams(
            backends,
            Arc::clone(&control),
            settings,
            &restored,
            fault.sink(),
        )?;

        // A restored loop is there, but silent until it's asked for.
        let state_machine = if restored.is_empty() {
            LoopStateMachine::new()
        } else {
            LoopStateMachine::stopped()
        };
        control.publish_state(state_machine.state());

        Ok(Self {
            control,
            state_machine,
            input_handler: InputHandler::new(Duration::from_millis(u64::from(
                settings.long_press_ms,
            ))),
            button_held: false,
            preroll: Preroll::new(Duration::from_millis(u64::from(settings.preroll_ms))),
            max_layers: settings.max_layers as usize,
            muted_layers: 0,
            mirror: LoopMirror::new(streams.captured, streams.sample_rate, restored),
            sample_rate: streams.sample_rate,
            config: settings.clone(),
            dir,
            now: Instant::now(),
            fault,
            underruns: 0,
            _stream: streams.stream,
        })
    }

    pub fn state(&self) -> LoopState {
        self.state_machine.state()
    }

    /// Why the audio stopped, if it did.
    ///
    /// Nothing here tries to recover: a stream that has failed is gone,
    /// and reopening means going back to Settings and choosing again -
    /// which may be choosing a different device, if the last one was
    /// unplugged.
    pub fn device_fault(&self) -> Option<String> {
        self.fault.message()
    }

    /// How many times the audio has fallen behind. Non-zero means the
    /// loop has audible gaps in it and the latency setting is too low
    /// for this machine - which the player can do something about, but
    /// only if told.
    pub fn underruns(&self) -> usize {
        self.underruns
    }

    pub fn is_long_press_active(&self) -> bool {
        self.input_handler.is_long_press_active()
    }

    /// How long the control has to be held to clear, for the hint line.
    pub fn long_press_secs(&self) -> f32 {
        self.input_handler.long_press_threshold().as_secs_f32()
    }

    /// Loop length in seconds (0.0 if empty) and playback position as a
    /// 0.0-1.0 fraction.
    pub fn loop_duration_and_progress(&self) -> (f32, f32) {
        self.control.loop_duration_and_progress(self.sample_rate)
    }

    pub fn layer_count(&self) -> usize {
        self.control.layer_count()
    }

    pub fn max_layers(&self) -> usize {
        self.max_layers
    }

    /// Whether the overdub control would do anything: there has to be a
    /// loop playing, and a layer left to record into.
    pub fn can_overdub(&self) -> bool {
        match self.state() {
            LoopState::Looping => self.layer_count() < self.max_layers,
            LoopState::Overdubbing => true,
            _ => false,
        }
    }

    /// The overdub control: opens a layer over the playing loop, or
    /// closes the one in progress.
    pub fn toggle_overdub(&mut self) {
        if !self.can_overdub() {
            return;
        }
        self.state_machine.toggle_overdub();
        self.control.publish_state(self.state_machine.state());
    }

    pub fn is_layer_muted(&self, index: usize) -> bool {
        self.muted_layers & (1 << index) != 0
    }

    /// Mutes a finished layer, or unmutes it. The take still being
    /// recorded can't be muted - it isn't a layer yet.
    pub fn toggle_layer_mute(&mut self, index: usize) {
        if index >= self.layer_count() {
            return;
        }
        self.muted_layers ^= 1 << index;
        self.control.publish_muted_layers(self.muted_layers);
    }

    /// Clears the mute bits from `count` up, so a layer recorded into a
    /// freed slot doesn't start out muted.
    fn forget_mutes_from(&mut self, count: usize) {
        self.muted_layers &= (1 << count) - 1;
        self.control.publish_muted_layers(self.muted_layers);
    }

    /// Only while nothing is being recorded - dropping a finished layer
    /// mid-take would be ambiguous about which one is meant.
    pub fn can_remove_layer(&self) -> bool {
        self.layer_count() > 0 && matches!(self.state(), LoopState::Looping | LoopState::Stopped)
    }

    /// Drops the newest layer. Dropping the only one leaves nothing to
    /// play, so the state goes back to Idle - but it isn't a clear: redo
    /// can still bring the loop back.
    pub fn remove_last_layer(&mut self) {
        if !self.can_remove_layer() {
            return;
        }
        let remaining = self.layer_count() - 1;
        self.control.request_remove_layer();
        self.forget_mutes_from(remaining);
        self.mirror.remove_last_layer();
        self.save_loop();
        if remaining == 0 {
            self.state_machine.clear();
            self.control.publish_state(self.state_machine.state());
        }
    }

    pub fn mode(&self) -> Mode {
        self.config.mode
    }

    /// Only while nothing plays or records. A stopped loop counts as
    /// still: a saved loop always comes back stopped, so requiring no
    /// loop at all would mean clearing it just to switch.
    pub fn can_switch_mode(&self) -> bool {
        matches!(self.state(), LoopState::Idle | LoopState::Stopped)
    }

    /// Flips between mini and extended, and saves it so the app reopens
    /// the way it was left. Going to mini unmutes every layer: mini has
    /// no layer row, so a mute there would be silence with no visible
    /// cause.
    pub fn toggle_mode(&mut self) {
        if !self.can_switch_mode() {
            return;
        }
        self.config.mode = match self.config.mode {
            Mode::Mini => Mode::Extended,
            Mode::Extended => Mode::Mini,
        };
        if self.config.mode == Mode::Mini {
            self.forget_mutes_from(0);
        }
        if let Err(err) = self.config.save_to(&config::config_path_in(&self.dir)) {
            eprintln!("could not save the mode: {err}");
        }
    }

    /// Whether redo would do anything: a layer removed since the last
    /// take, and nothing being recorded over it. Idle counts - removing
    /// the only layer lands there.
    pub fn can_redo_layer(&self) -> bool {
        self.mirror.can_redo()
            && matches!(
                self.state(),
                LoopState::Idle | LoopState::Looping | LoopState::Stopped
            )
    }

    /// Brings back the layer "remove" dropped, unmuted. From Idle the
    /// loop comes back stopped, like a saved one at startup. A long-press
    /// clear is final: it forgets what redo could bring back.
    pub fn redo_layer(&mut self) {
        if !self.can_redo_layer() {
            return;
        }
        self.state_machine.restore();
        self.control.publish_state(self.state_machine.state());
        self.control.request_redo_layer();
        self.mirror.redo_layer();
        self.save_loop();
    }

    pub fn set_button_held(&mut self, held: bool) {
        self.button_held = held;
    }

    /// Seconds left of the pre-roll countdown, or 0.0 when one isn't
    /// running.
    pub fn preroll_remaining_secs(&self) -> f32 {
        self.preroll.remaining_secs(self.now)
    }

    /// How far through the pre-roll the countdown has got, 0.0-1.0.
    pub fn preroll_progress(&self) -> f32 {
        self.preroll.progress(self.now)
    }

    /// Call once per frame. Folds the spacebar and the button's latch into
    /// one `InputHandler`, so the two can't desync.
    pub fn tick(&mut self, key_held: bool, now: Instant) {
        self.now = now;
        self.log_underruns();

        let event = self.input_handler.update(key_held || self.button_held, now);
        self.apply(event, now);

        // Deliberately after the input: if a press meant to call the
        // pre-roll off lands on the same frame the countdown runs out,
        // the press should win.
        if self.preroll.take_if_elapsed(now) {
            self.state_machine.finish_arming();
            self.control.publish_state(self.state_machine.state());
        }

        if self.control.take_capture_lost() > 0 {
            self.mirror.note_lost_samples();
        }
        if self.mirror.tick(
            self.state_machine.state(),
            self.control.loop_len(),
            self.control.take_start(),
        ) {
            self.save_loop();
        }
    }

    /// Writes the loop out, so closing the app doesn't lose it. A failure
    /// is reported and otherwise ignored: it isn't worth interrupting
    /// playing over.
    fn save_loop(&self) {
        if let Err(err) = self.mirror.save(&config::loop_dir_in(&self.dir)) {
            eprintln!("could not save the loop: {err}");
        }
    }

    /// Publishes the resulting state in the same step as changing it, so
    /// that pairing can't be forgotten at a call site.
    fn apply(&mut self, event: InputEvent, now: Instant) {
        match event {
            InputEvent::ShortPress => {
                // A pre-roll is only for the first recording - once a
                // loop is playing you're already in time with it.
                if self.state() == LoopState::Idle && self.preroll.is_enabled() {
                    self.state_machine.arm();
                    self.preroll.start(now);
                } else {
                    self.state_machine.press();
                    self.preroll.cancel();
                }
                self.control.publish_state(self.state_machine.state());
            }
            InputEvent::LongPressClear => self.clear(),
            InputEvent::None => {}
        }
    }

    fn clear(&mut self) {
        self.state_machine.clear();
        self.preroll.cancel();
        self.control.publish_state(self.state_machine.state());
        self.control.request_clear();
        self.forget_mutes_from(0);
        self.mirror.clear();
        loop_mirror::delete(&config::loop_dir_in(&self.dir));
    }

    /// Drained here rather than in the callbacks, since stdio isn't
    /// real-time safe. Kept as well as logged: a release build has no
    /// console, so the count is what the screen has to go on.
    fn log_underruns(&mut self) {
        let (input_underruns, output_underruns) = self.control.take_underrun_counts();
        self.underruns += input_underruns + output_underruns;
        if input_underruns > 0 {
            eprintln!("input stream fell behind {input_underruns} time(s): try increasing latency");
        }
        if output_underruns > 0 {
            eprintln!(
                "output stream fell behind {output_underruns} time(s): try increasing latency"
            );
        }
    }
}

#[cfg(test)]
#[path = "looper_tests.rs"]
mod tests;
