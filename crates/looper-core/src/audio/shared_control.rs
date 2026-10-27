use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::config::StopMode;
use crate::state_machine::LoopState;

/// Lock-free relay across the UI/audio thread boundary. The state machine
/// stays single-owner on the UI thread (`looper.rs`); only its resulting
/// value and a one-shot clear flag cross, via atomics, never a lock.
pub struct SharedControl {
    state: AtomicU8,
    clear_requested: AtomicBool,
    remove_layer_requested: AtomicBool,
    redo_layer_requested: AtomicBool,
    /// Set just before a state change that ends a take, when the take is
    /// to be thrown away rather than kept - a double press's first half.
    abandon_take_requested: AtomicBool,
    /// Ends the overdub in progress and opens the next one, with no state
    /// change to carry it: dubbing goes on, into a layer of its own.
    next_layer_requested: AtomicBool,
    /// A stop the audio thread carries out itself, to the frame: 0 for
    /// none, else a `StopMode`. A level, not a one-shot - it's acted on
    /// every block it's set and the loop plays, and cleared (cancelled)
    /// before any state leaving it is published.
    pending_stop: AtomicU8,
    /// Set by the audio thread when that stop has run its course: the
    /// loop end reached, or the fade at silence. The UI thread publishes
    /// Stopped on seeing it.
    stop_done: AtomicBool,
    // Telemetry the other way, audio -> UI: duration, progress bar, and
    // how many layers are stacked up.
    loop_len: AtomicUsize,
    play_pos: AtomicUsize,
    layer_count: AtomicUsize,
    /// Whether the stack still has a take open. Published last, so a
    /// reader that sees it closed also sees every take's end report: the
    /// UI thread holds off remove and redo until then.
    take_open: AtomicBool,
    /// Samples that never reached the UI thread's copy, because its ring
    /// was full. Means that copy no longer matches what's playing.
    capture_lost: AtomicUsize,
    // Also audio -> UI. Monotonic counts with no dependent data, so
    // Relaxed is enough, unlike `state`/`play_pos`. Logged from the UI
    // thread - stdio in the callback would add I/O exactly when it's
    // already falling behind.
    input_underruns: AtomicUsize,
    output_underruns: AtomicUsize,
    // Loop playback gain, UI -> audio like `state`. A live atomic rather
    // than a plain value on the audio side so it can become adjustable
    // without a restart.
    volume_pct: AtomicU32,
    /// The click's level, UI -> audio, likewise.
    click_volume_pct: AtomicU32,
    /// Which layers are muted, one bit each, UI -> audio. A bitmask so
    /// every layer's mute crosses in one atomic; `MAX_LAYERS_RANGE` tops
    /// out well inside 32.
    muted_layers: AtomicU32,
    /// The metronome toggle, UI -> audio.
    metronome_on: AtomicBool,
    /// Its tempo, UI -> audio, live: nudged from the looper screen, or
    /// taken from a loop's length. An `f64`'s bits, since a loop's tempo
    /// is rarely a whole number.
    metronome_bpm: AtomicU64,
}

impl SharedControl {
    pub fn new(volume_pct: u32, metronome_bpm: u32) -> Self {
        Self {
            state: AtomicU8::new(LoopState::Idle as u8),
            clear_requested: AtomicBool::new(false),
            remove_layer_requested: AtomicBool::new(false),
            redo_layer_requested: AtomicBool::new(false),
            abandon_take_requested: AtomicBool::new(false),
            next_layer_requested: AtomicBool::new(false),
            pending_stop: AtomicU8::new(0),
            stop_done: AtomicBool::new(false),
            loop_len: AtomicUsize::new(0),
            play_pos: AtomicUsize::new(0),
            layer_count: AtomicUsize::new(0),
            take_open: AtomicBool::new(false),

            capture_lost: AtomicUsize::new(0),
            input_underruns: AtomicUsize::new(0),
            output_underruns: AtomicUsize::new(0),
            volume_pct: AtomicU32::new(volume_pct),
            click_volume_pct: AtomicU32::new(100),
            muted_layers: AtomicU32::new(0),
            metronome_on: AtomicBool::new(false),
            metronome_bpm: AtomicU64::new(f64::from(metronome_bpm).to_bits()),
        }
    }

    pub fn publish_metronome_on(&self, on: bool) {
        self.metronome_on.store(on, Ordering::Release);
    }

    pub fn publish_metronome_bpm(&self, bpm: f64) {
        self.metronome_bpm.store(bpm.to_bits(), Ordering::Release);
    }

    pub fn publish_volume_pct(&self, pct: u32) {
        self.volume_pct.store(pct, Ordering::Release);
    }

    pub fn publish_click_volume_pct(&self, pct: u32) {
        self.click_volume_pct.store(pct, Ordering::Release);
    }

    pub fn publish_muted_layers(&self, muted: u32) {
        self.muted_layers.store(muted, Ordering::Release);
    }

    pub fn publish_state(&self, state: LoopState) {
        self.state.store(state as u8, Ordering::Release);
    }

    pub fn request_clear(&self) {
        self.clear_requested.store(true, Ordering::Release);
    }

    /// Drops the newest layer. One-shot, like `request_clear`.
    pub fn request_remove_layer(&self) {
        self.remove_layer_requested.store(true, Ordering::Release);
    }

    /// Brings back the layer last removed. One-shot, like
    /// `request_remove_layer`.
    pub fn request_redo_layer(&self) {
        self.redo_layer_requested.store(true, Ordering::Release);
    }

    /// The take the next state change ends is dropped, not kept. Set
    /// *before* publishing that change: the audio thread reads it once it
    /// sees the change, so seeing the change means seeing this too.
    pub fn request_abandon_take(&self) {
        self.abandon_take_requested.store(true, Ordering::Release);
    }

    /// One-shot, like `request_clear`. Only means anything while
    /// overdubbing; the audio thread drops it otherwise.
    pub fn request_next_layer(&self) {
        self.next_layer_requested.store(true, Ordering::Release);
    }

    /// Asks the audio thread to stop the playing loop its own way. The
    /// done flag is cleared first, so one left from an earlier stop
    /// can't end this one at once.
    pub fn request_stop(&self, mode: StopMode) {
        self.stop_done.store(false, Ordering::Release);
        let level = match mode {
            StopMode::Immediate => 0,
            StopMode::LoopEnd => 1,
            StopMode::FadeOut => 2,
        };
        self.pending_stop.store(level, Ordering::Release);
    }

    /// Before publishing a state that leaves the stop behind - so the
    /// audio thread, seeing that state, sees this too.
    pub fn cancel_stop(&self) {
        self.pending_stop.store(0, Ordering::Release);
    }

    pub fn take_stop_done(&self) -> bool {
        self.stop_done.swap(false, Ordering::AcqRel)
    }

    pub fn layer_count(&self) -> usize {
        self.layer_count.load(Ordering::Acquire)
    }

    pub fn loop_len(&self) -> usize {
        self.loop_len.load(Ordering::Acquire)
    }

    pub fn take_open(&self) -> bool {
        self.take_open.load(Ordering::Acquire)
    }

    /// Drains the count of samples lost since the last call.
    pub fn take_capture_lost(&self) -> usize {
        self.capture_lost.swap(0, Ordering::Relaxed)
    }

    /// Loop duration in seconds (0.0 if empty) and playback position as a
    /// 0.0-1.0 fraction. The buffer is mono, so no channel count is needed.
    pub fn loop_duration_and_progress(&self, sample_rate: u32) -> (f32, f32) {
        let loop_len = self.loop_len.load(Ordering::Acquire);
        let play_pos = self.play_pos.load(Ordering::Acquire);
        let samples_per_second = (sample_rate as usize).max(1);

        let duration_secs = loop_len as f32 / samples_per_second as f32;
        let progress_fraction = if loop_len > 0 {
            play_pos as f32 / loop_len as f32
        } else {
            0.0
        };
        (duration_secs, progress_fraction)
    }

    pub(super) fn load_state(&self) -> LoopState {
        match self.state.load(Ordering::Acquire) {
            0 => LoopState::Idle,
            1 => LoopState::Recording,
            2 => LoopState::Looping,
            3 => LoopState::Stopped,
            4 => LoopState::Overdubbing,
            _ => LoopState::Arming,
        }
    }

    pub(super) fn take_clear_request(&self) -> bool {
        self.clear_requested.swap(false, Ordering::AcqRel)
    }

    pub(super) fn take_remove_layer_request(&self) -> bool {
        self.remove_layer_requested.swap(false, Ordering::AcqRel)
    }

    pub(super) fn take_abandon_take_request(&self) -> bool {
        self.abandon_take_requested.swap(false, Ordering::AcqRel)
    }

    pub(super) fn take_next_layer_request(&self) -> bool {
        self.next_layer_requested.swap(false, Ordering::AcqRel)
    }

    pub(super) fn pending_stop(&self) -> Option<StopMode> {
        match self.pending_stop.load(Ordering::Acquire) {
            1 => Some(StopMode::LoopEnd),
            2 => Some(StopMode::FadeOut),
            _ => None,
        }
    }

    pub(super) fn note_stop_done(&self) {
        self.stop_done.store(true, Ordering::Release);
    }

    pub(super) fn take_redo_layer_request(&self) -> bool {
        self.redo_layer_requested.swap(false, Ordering::AcqRel)
    }

    pub(super) fn note_capture_lost(&self, samples: usize) {
        self.capture_lost.fetch_add(samples, Ordering::Relaxed);
    }

    /// `take_open` last, so a reader that sees it closed also sees the
    /// length the take closed on.
    pub(super) fn publish_telemetry(&self, len: usize, pos: usize, layers: usize, take_open: bool) {
        self.loop_len.store(len, Ordering::Release);
        self.play_pos.store(pos, Ordering::Release);
        self.layer_count.store(layers, Ordering::Release);
        self.take_open.store(take_open, Ordering::Release);
    }

    pub(super) fn metronome_on(&self) -> bool {
        self.metronome_on.load(Ordering::Acquire)
    }

    pub(super) fn metronome_bpm(&self) -> f64 {
        f64::from_bits(self.metronome_bpm.load(Ordering::Acquire))
    }

    pub(super) fn muted_layers(&self) -> u32 {
        self.muted_layers.load(Ordering::Acquire)
    }

    pub(super) fn volume_pct(&self) -> u32 {
        self.volume_pct.load(Ordering::Acquire)
    }

    pub(super) fn click_volume_pct(&self) -> u32 {
        self.click_volume_pct.load(Ordering::Acquire)
    }

    pub(super) fn note_input_underrun(&self) {
        self.input_underruns.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn note_output_underrun(&self) {
        self.output_underruns.fetch_add(1, Ordering::Relaxed);
    }

    /// Drains the counts accumulated since the last call, `(input, output)`.
    pub fn take_underrun_counts(&self) -> (usize, usize) {
        (
            self.input_underruns.swap(0, Ordering::Relaxed),
            self.output_underruns.swap(0, Ordering::Relaxed),
        )
    }
}
