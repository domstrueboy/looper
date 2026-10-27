use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use looper_hal::{AudioStream, Backends};

use crate::audio::engine;
use crate::audio::fault::StreamFault;
use crate::audio::scheduler::{Pattern, Scheduler};
use crate::audio::shared_control::SharedControl;
use crate::state_machine::{LoopState, LoopStateMachine};
use crate::config::{self, AppConfig, METRONOME_BPM_RANGE, Mode, RecordMode};
use crate::input::{InputEvent, InputHandler};
use crate::loop_mirror::{self, LoopMirror};
use crate::preroll::Preroll;
use crate::tempo::LoopTempo;

/// 4/4, as the metronome counts it.
const BEATS_PER_BAR: u32 = 4;

/// The metronome's range, for a tempo that isn't a whole number.
fn bpm_range() -> RangeInclusive<f64> {
    f64::from(*METRONOME_BPM_RANGE.start())..=f64::from(*METRONOME_BPM_RANGE.end())
}

/// Where a loop's tempo comes from - worked out once its length is.
#[derive(Clone, Copy)]
enum TempoSource {
    /// Played without the click: guessed from the length.
    Guess,
    /// Played against the click, whose bar this is: the click's tempo.
    Click(f64),
    /// Restored, with its beat count saved beside it.
    Saved(u32),
}

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
    /// The stop control's own handler, so it has a hold of its own:
    /// press stops, hold clears. Its key and button, likewise.
    stop_input: InputHandler,
    stop_key_held: bool,
    stop_button_held: bool,
    /// The countdown before the first recording starts.
    preroll: Preroll,
    /// Set when stop was pressed with the metronome on and the nearest
    /// bar line still ahead: recording carries on to it, counted in
    /// frames recorded. The audio thread rounds the take to the bar
    /// either way; this only makes sure there's a bar's worth to round.
    stop_at: Option<usize>,
    /// When a count-in began - set instead of `preroll` while the
    /// metronome is on. Only for showing it: the take opens on the audio
    /// thread at the bar line, and this side follows.
    count_in: Option<Instant>,
    /// The state the last single press acted from. A double press is two
    /// presses with the first already acted on, so it needs to know what
    /// that first one changed to take it back.
    pressed_from: Option<LoopState>,
    /// The loop's tempo, worked out from its length once it has one.
    /// While a loop is there, the click follows this rather than the
    /// configured tempo - anything else would drift off the loop.
    loop_tempo: Option<LoopTempo>,
    /// Where the loop's tempo will come from, until its length is known.
    tempo_pending: Option<TempoSource>,
    /// Kept here as well as in the layer stack, which lives on the audio
    /// thread out of reach - the UI needs it to show "n/max" and to know
    /// when overdubbing is still possible.
    max_layers: usize,
    /// Which layers are muted, one bit each - owned here and published
    /// to the audio thread whole. Not saved: a loop reopens with every
    /// layer playing.
    muted_layers: u32,
    metronome_on: bool,
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
    /// See `take_in_flight()`.
    take_in_flight: bool,
    /// A hold asked for an undo, made once the take its press opened is
    /// seen dropped - until then the stack and the copy could disagree
    /// on which layer is last.
    undo_pending: bool,
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
        let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
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
        let restored_empty = restored.is_empty();
        let state_machine = if restored_empty {
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
            stop_input: InputHandler::new(Duration::from_millis(u64::from(
                settings.long_press_ms,
            ))),
            stop_key_held: false,
            stop_button_held: false,
            preroll: Preroll::new(Duration::from_millis(u64::from(settings.preroll_ms))),
            stop_at: None,
            count_in: None,
            pressed_from: None,
            loop_tempo: None,
            // A saved loop has a length but no tempo yet; guessed once
            // the audio thread reports the length.
            tempo_pending: (!restored_empty).then(|| {
                match loop_mirror::load_beats(&config::loop_dir_in(&dir)) {
                    Some(beats) => TempoSource::Saved(beats),
                    None => TempoSource::Guess,
                }
            }),
            max_layers: settings.max_layers as usize,
            muted_layers: 0,
            metronome_on: false,
            mirror: LoopMirror::new(streams.captured, streams.sample_rate, restored),
            sample_rate: streams.sample_rate,
            config: settings.clone(),
            dir,
            now: Instant::now(),
            fault,
            underruns: 0,
            take_in_flight: false,
            undo_pending: false,
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

    /// The main control has been held long enough to undo.
    pub fn is_long_press_active(&self) -> bool {
        self.input_handler.is_long_press_active()
    }

    /// The stop control has been held long enough to clear.
    pub fn is_stop_long_press_active(&self) -> bool {
        self.stop_input.is_long_press_active()
    }

    /// How long either control has to be held, for the hint line.
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


    /// Whether the stop control would do anything: something has to be
    /// playing, recording or counting in.
    pub fn can_stop(&self) -> bool {
        !matches!(self.state(), LoopState::Idle | LoopState::Stopped)
    }

    /// The stop control - the second pedal, and what a double press does
    /// on the first. A take or a dub ends and is kept, a count-in is
    /// called off. In REC→DUB, where the main control never leaves
    /// dubbing, it steps down: dubbing to playing, playing to stopped.
    pub fn stop(&mut self) {
        match self.state() {
            LoopState::Idle | LoopState::Stopped => return,
            // Now, not at the bar line: the length is still rounded,
            // there's just nothing left to play out.
            LoopState::Recording => {
                self.stop_at = None;
                self.end_take(false);
                self.state_machine.press();
            }
            LoopState::Arming => {
                self.state_machine.press();
                self.preroll.cancel();
                self.count_in = None;
            }
            LoopState::Overdubbing if self.config.record_mode == RecordMode::RecDub => {
                self.state_machine.toggle_overdub();
            }
            LoopState::Looping | LoopState::Overdubbing => self.state_machine.press(),
        }
        self.control.publish_state(self.state_machine.state());
    }

    pub fn is_metronome_on(&self) -> bool {
        self.metronome_on
    }

    /// The tempo the click is playing at: the loop's while there is one,
    /// otherwise the configured one.
    pub fn tempo(&self) -> f64 {
        match self.active_loop_tempo() {
            Some(tempo) => tempo.bpm(),
            None => f64::from(self.config.metronome_bpm),
        }
    }

    /// Whether the tempo is the loop's - which is fixed by its length,
    /// so it can only be halved or doubled, not nudged.
    pub fn tempo_is_the_loops(&self) -> bool {
        self.active_loop_tempo().is_some()
    }

    /// A removed loop that redo could bring back keeps its tempo, but
    /// with nothing playing the click is the configured one again.
    fn active_loop_tempo(&self) -> Option<LoopTempo> {
        self.loop_tempo.filter(|_| self.layer_count() > 0)
    }

    pub fn can_halve_tempo(&self) -> bool {
        self.active_loop_tempo()
            .and_then(|tempo| tempo.halved(bpm_range()))
            .is_some()
    }

    pub fn can_double_tempo(&self) -> bool {
        self.active_loop_tempo()
            .and_then(|tempo| tempo.doubled(bpm_range()))
            .is_some()
    }

    /// For a guess that came out twice too fast: same loop, half the
    /// beats.
    pub fn halve_tempo(&mut self) {
        if let Some(halved) = self.active_loop_tempo().and_then(|t| t.halved(bpm_range())) {
            self.loop_tempo = Some(halved);
            self.control.publish_metronome_bpm(self.tempo());
            self.save_tempo();
        }
    }

    pub fn double_tempo(&mut self) {
        if let Some(doubled) = self.active_loop_tempo().and_then(|t| t.doubled(bpm_range())) {
            self.loop_tempo = Some(doubled);
            self.control.publish_metronome_bpm(self.tempo());
            self.save_tempo();
        }
    }

    /// Off at every start: a click nobody asked for, the moment the app
    /// opens, is worse than pressing M. Turning it off mid count-in calls
    /// the count-in off - there'd be no bar line left to start on.
    pub fn toggle_metronome(&mut self) {
        self.metronome_on = !self.metronome_on;
        self.control.publish_metronome_on(self.metronome_on);
        if !self.metronome_on && self.count_in.take().is_some() {
            self.state_machine.press();
            self.control.publish_state(self.state_machine.state());
        }
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

    /// Whether the audio thread still had a take open as of this frame.
    /// While it does, the take's end hasn't reached the copy yet, so the
    /// copy and the stack can disagree on which layer is last - remove or
    /// redo then would act on different layers. A frame or so at most.
    pub fn take_in_flight(&self) -> bool {
        self.take_in_flight
    }

    /// Only while nothing is being recorded - dropping a finished layer
    /// mid-take would be ambiguous about which one is meant. Never the
    /// first take: undo takes back layers, and emptying the loop is a
    /// clear's job.
    pub fn can_remove_layer(&self) -> bool {
        self.layer_count() > 1
            && matches!(self.state(), LoopState::Looping | LoopState::Stopped)
            && !self.take_in_flight
    }

    /// Drops the newest layer.
    pub fn remove_last_layer(&mut self) {
        if !self.can_remove_layer() {
            return;
        }
        let remaining = self.layer_count() - 1;
        self.control.request_remove_layer();
        self.forget_mutes_from(remaining);
        self.mirror.remove_last_layer();
        self.save_loop();
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
        self.save_config();
    }

    /// Nudges the configured tempo by `delta` BPM, within its range,
    /// while it plays. Saved, so Settings and the next start agree with
    /// what was last heard. Not while a loop sets the tempo.
    pub fn change_metronome_bpm(&mut self, delta: i32) {
        if self.tempo_is_the_loops() {
            return;
        }
        let bpm = self
            .config
            .metronome_bpm
            .saturating_add_signed(delta)
            .clamp(*METRONOME_BPM_RANGE.start(), *METRONOME_BPM_RANGE.end());
        if bpm == self.config.metronome_bpm {
            return;
        }
        self.config.metronome_bpm = bpm;
        self.control.publish_metronome_bpm(self.tempo());
        self.save_config();
    }

    /// For the settings changed from this screen rather than from
    /// Settings. Reported and otherwise ignored, like a failed loop save.
    fn save_config(&self) {
        if let Err(err) = self.config.save_to(&config::config_path_in(&self.dir)) {
            eprintln!("could not save the config: {err}");
        }
    }

    /// Whether redo would do anything: a layer removed since the last
    /// take, and nothing being recorded over it.
    pub fn can_redo_layer(&self) -> bool {
        self.mirror.can_redo()
            && matches!(self.state(), LoopState::Looping | LoopState::Stopped)
            && !self.take_in_flight
    }

    /// Brings back the layer "remove" dropped, unmuted. A long-press
    /// clear is final: it forgets what redo could bring back.
    pub fn redo_layer(&mut self) {
        if !self.can_redo_layer() {
            return;
        }
        self.control.request_redo_layer();
        self.mirror.redo_layer();
        self.save_loop();
    }

    pub fn set_button_held(&mut self, held: bool) {
        self.button_held = held;
    }

    /// The S key, read by the app each frame as the spacebar is.
    pub fn set_stop_key_held(&mut self, held: bool) {
        self.stop_key_held = held;
    }

    /// Latched like `set_button_held`, for the same reason.
    pub fn set_stop_button_held(&mut self, held: bool) {
        self.stop_button_held = held;
    }

    /// Whether the stop control would do anything, a hold included:
    /// stopped, there's still a loop to clear.
    pub fn can_stop_or_clear(&self) -> bool {
        self.state() != LoopState::Idle
    }

    /// Whether stop was pressed and recording is carrying on to the bar
    /// line - so the screen can say why it hasn't stopped yet.
    pub fn is_finishing_bar(&self) -> bool {
        self.stop_at.is_some()
    }

    /// Seconds left of the pre-roll countdown, or 0.0 when one isn't
    /// running.
    pub fn preroll_remaining_secs(&self) -> f32 {
        match self.count_in {
            Some(start) => (self.bar_secs() - self.secs_since(start)).max(0.0),
            None => self.preroll.remaining_secs(self.now),
        }
    }

    /// How far through the pre-roll the countdown has got, 0.0-1.0.
    pub fn preroll_progress(&self) -> f32 {
        match self.count_in {
            Some(start) => (self.secs_since(start) / self.bar_secs()).min(1.0),
            None => self.preroll.progress(self.now),
        }
    }

    fn secs_since(&self, start: Instant) -> f32 {
        self.now.saturating_duration_since(start).as_secs_f32()
    }

    fn bar_secs(&self) -> f32 {
        (self.bar_frames() / f64::from(self.sample_rate)) as f32
    }

    /// The same bar the audio thread counts and rounds to.
    fn bar_frames(&self) -> f64 {
        Scheduler::new(
            self.sample_rate,
            f64::from(self.config.metronome_bpm),
            Pattern::METRONOME,
        )
        .frames_per_bar()
    }

    /// Call once per frame. Folds the spacebar and the button's latch into
    /// one `InputHandler`, so the two can't desync.
    pub fn tick(&mut self, key_held: bool, now: Instant) {
        self.now = now;
        self.log_underruns();

        let event = self.input_handler.update(key_held || self.button_held, now);
        self.apply(event, now);
        // A second stop is the same as one, so a double is two presses.
        match self
            .stop_input
            .update(self.stop_key_held || self.stop_button_held, now)
        {
            InputEvent::Press | InputEvent::DoublePress => self.stop(),
            InputEvent::LongPress => self.clear(),
            InputEvent::None => {}
        }

        // Deliberately after the input: if a press meant to call the
        // pre-roll off lands on the same frame the countdown runs out,
        // the press should win.
        if self.preroll.take_if_elapsed(now) {
            self.state_machine.finish_arming();
            self.control.publish_state(self.state_machine.state());
        }

        // The audio thread opens a count-in's take on the bar line; the
        // take growing is the sign it has.
        if self.count_in.is_some() && self.control.loop_len() > 0 {
            self.count_in = None;
            self.state_machine.finish_arming();
            self.control.publish_state(self.state_machine.state());
        }

        if let Some(target) = self.stop_at
            && self.control.loop_len() >= target
        {
            self.stop_at = None;
            self.end_take(self.dubs_after_take());
            self.control.publish_state(self.state_machine.state());
        }

        if let Some(source) = self.tempo_pending
            && self.state() != LoopState::Recording
            && self.layer_count() > 0
        {
            self.tempo_pending = None;
            let len = self.control.loop_len();
            let guess = LoopTempo::guess(len, self.sample_rate);
            self.loop_tempo = Some(match source {
                TempoSource::Guess => guess,
                TempoSource::Click(bar) => {
                    LoopTempo::from_bars(len, self.sample_rate, bar, BEATS_PER_BAR)
                }
                // A hand-edited file can say anything; a tempo the click
                // can't play falls back to the guess.
                TempoSource::Saved(beats) => Some(LoopTempo::with_beats(beats, len, self.sample_rate))
                    .filter(|tempo| bpm_range().contains(&tempo.bpm()))
                    .unwrap_or(guess),
            });
            self.save_tempo();
        }
        // Every frame rather than at each change: which tempo applies
        // turns on the layer count, which the audio thread moves.
        self.control.publish_metronome_bpm(self.tempo());

        if self.control.take_capture_lost() > 0 {
            self.mirror.note_lost_samples();
        }
        // Before the copy reads the take ends: seen closed here, the end
        // of every take is already on its ring. See `take_in_flight`.
        self.take_in_flight = self.control.take_open();
        if self.mirror.tick(self.state_machine.state()) {
            self.save_loop();
        }
        if self.undo_pending && !self.take_in_flight {
            self.undo_pending = false;
            self.remove_last_layer();
        }
    }

    /// Beside the loop, so it comes back with the tempo it had rather
    /// than a fresh guess - a halved one included. Reported and otherwise
    /// ignored, like a failed loop save.
    fn save_tempo(&self) {
        let Some(tempo) = self.loop_tempo else {
            return;
        };
        if let Err(err) = loop_mirror::save_beats(&config::loop_dir_in(&self.dir), tempo.beats()) {
            eprintln!("could not save the tempo: {err}");
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
            InputEvent::Press => self.press(now),
            InputEvent::DoublePress => self.double_press(),
            InputEvent::LongPress => self.hold(),
            InputEvent::None => {}
        }
    }

    /// Never stops - that's S, or a double press. From empty it records,
    /// then walks on through the record mode's cycle; stopped, it plays.
    fn press(&mut self, now: Instant) {
        self.pressed_from = Some(self.state());
        // A second press while waiting for the bar line means now: the
        // take is still rounded, the rest plays silent.
        if self.state() == LoopState::Recording && self.stop_at.is_none() {
            self.stop_at = self.bar_line_ahead();
            if self.stop_at.is_some() {
                return;
            }
        }
        self.stop_at = None;
        match self.state() {
            // A pre-roll is only for the first recording - once a loop
            // is playing you're already in time with it. With the
            // metronome on it's a bar's count-in instead of the delay.
            LoopState::Idle if self.metronome_on => {
                self.state_machine.arm();
                self.count_in = Some(now);
            }
            LoopState::Idle if self.preroll.is_enabled() => {
                self.state_machine.arm();
                self.preroll.start(now);
            }
            LoopState::Recording => self.end_take(self.dubs_after_take()),
            // With every layer used there's nothing to dub into, and a
            // press does nothing.
            LoopState::Looping => {
                if self.can_overdub() {
                    self.state_machine.toggle_overdub();
                }
            }
            // No state change to publish: dubbing goes on.
            LoopState::Overdubbing if self.dubs_on() => {
                return self.control.request_next_layer();
            }
            LoopState::Overdubbing => self.state_machine.toggle_overdub(),
            // Records with no delay, calls a count-in off, plays a
            // stopped loop.
            LoopState::Idle | LoopState::Arming | LoopState::Stopped => {
                self.state_machine.press();
                self.preroll.cancel();
                self.count_in = None;
            }
        }
        self.control.publish_state(self.state_machine.state());
    }

    /// What S does - but the first half already acted as it went down,
    /// so whatever that started is taken back first: a dub it opened is
    /// dropped rather than left as a sliver of a layer, and a take or a
    /// count-in it began on an empty looper leaves it empty.
    fn double_press(&mut self) {
        let from = self.pressed_from.take();
        match from {
            Some(LoopState::Idle) => return self.clear(),
            // The first half already called the count-in off.
            Some(LoopState::Arming) => return,
            _ => {}
        }
        // A dub going now can only be the first half's: one going
        // before it, it closed - or in REC→DUB split, opening this one.
        if self.state() == LoopState::Overdubbing {
            self.control.request_abandon_take();
            self.state_machine.toggle_overdub();
        }
        // REC→DUB's stop from dubbing is to playing, where the take-back
        // has already left it.
        if from == Some(LoopState::Overdubbing) && self.config.record_mode == RecordMode::RecDub {
            return self.control.publish_state(self.state_machine.state());
        }
        self.stop();
    }

    /// Undo, one layer a hold. The press it started with has already
    /// acted, so that's taken back first. Held while dubbing, the dub in
    /// progress is the layer undone; held while recording the first
    /// take, the take is - leaving the looper empty. A finished first
    /// take never is: that's a clear, on the stop control.
    fn hold(&mut self) {
        let from = self.pressed_from.take();
        match from {
            Some(LoopState::Idle | LoopState::Recording) => return self.clear(),
            // The press called a count-in off: nothing left to undo.
            Some(LoopState::Arming) => return,
            _ => {}
        }
        // Only the press can have opened a dub that's going now - from
        // playing, or in REC→DUB as the next layer, the one before it
        // being what's undone.
        if self.state() == LoopState::Overdubbing {
            self.control.request_abandon_take();
            self.state_machine.toggle_overdub();
        }
        // The press played it.
        if from == Some(LoopState::Stopped) {
            self.state_machine.press();
        }
        self.control.publish_state(self.state_machine.state());
        self.undo_pending = true;
    }

    /// Where a take stopped now would be rounded up to, if the metronome
    /// is on and that's still ahead - in frames recorded, the same count
    /// the audio thread rounds. Nearer the bar line behind, or past what
    /// a loop can hold, there's nothing to wait for.
    fn bar_line_ahead(&self) -> Option<usize> {
        if !self.metronome_on {
            return None;
        }
        let bar = self.bar_frames();
        let played = self.control.loop_len() as f64;
        let target = ((played / bar).round().max(1.0) * bar).round();
        let capacity = f64::from(self.config.max_loop_secs) * f64::from(self.sample_rate);
        (target > played && target <= capacity).then_some(target as usize)
    }

    /// Ends a first take, noting what its tempo will come from: the
    /// length isn't fixed until the audio thread has handled the stop.
    /// `into_dub` goes straight on into an overdub - one state change,
    /// which the audio thread takes as "close the take, fix the length,
    /// open a layer at the top". The caller publishes.
    fn end_take(&mut self, into_dub: bool) {
        self.tempo_pending = Some(if self.metronome_on {
            TempoSource::Click(self.bar_frames())
        } else {
            TempoSource::Guess
        });
        self.loop_tempo = None;
        self.state_machine.press();
        if into_dub {
            self.state_machine.toggle_overdub();
        }
    }

    pub fn record_mode(&self) -> RecordMode {
        self.config.record_mode
    }

    /// Whether ending the take goes straight on into a dub. Not with one
    /// layer allowed: there'd be nothing to dub into.
    fn dubs_after_take(&self) -> bool {
        self.config.record_mode.dubs_after_take() && self.max_layers > 1
    }

    /// Whether a press now keeps the dub in progress and dubs the next
    /// layer, rather than plays: REC→DUB, with a layer left after the
    /// one being recorded.
    pub fn dubs_on(&self) -> bool {
        self.state() == LoopState::Overdubbing
            && self.config.record_mode == RecordMode::RecDub
            && self.layer_count() + 2 <= self.max_layers
    }

    fn clear(&mut self) {
        self.undo_pending = false;
        self.stop_at = None;
        self.count_in = None;
        self.loop_tempo = None;
        self.tempo_pending = None;
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
