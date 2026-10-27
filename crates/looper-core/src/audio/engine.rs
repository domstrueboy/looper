//! The audio path: what happens to samples between the device's two
//! callbacks. Nothing here opens a device or knows what one is - the
//! backend hands buffers in and takes them out again.

use std::sync::Arc;

use looper_hal::{
    AudioStream, Backends, DeviceInfo, Direction, ErrorSink, InputProcessor, MAX_BLOCK_FRAMES,
    OutputProcessor, StreamRequest,
};
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Producer, Split},
};

use super::loop_stack::{LoopStack, OpenTake};
use super::metronome::Metronome;
use super::shared_control::SharedControl;
use crate::config::{AppConfig, RecordMode, RecordStart, StopMode};
use crate::loop_mirror::{Capture, TakeEnd};
use crate::state_machine::LoopState;

/// Bounds one callback's worth of work at a fixed size, so the scratch
/// buffers can be taken up front.
///
/// The same bound the backend promises, rather than a second opinion
/// about it. Kept as a loop of our own as well, because the tests drive
/// these paths directly - without a backend in front of them, nothing
/// else holds the buffers to a size the scratch can take.
const SCRATCH_CAPACITY: usize = MAX_BLOCK_FRAMES;

/// Seconds of captured audio the UI thread's copy can fall behind by
/// before samples start being dropped. Generous: losing any means the
/// recorded loop can't be saved.
const CAPTURE_BUFFER_SECONDS: usize = 2;

/// Take ends the UI thread's copy can fall behind by.
const TAKE_ENDS_CAPACITY: usize = 64;

/// How far before the detect level is crossed a detected take opens: a
/// pick's attack takes a few milliseconds to climb to it.
const DETECT_LOOKBACK_MS: usize = 10;

/// Holds the recorded signal back to where the monitored one is.
///
/// The dry path carries `latency_frames` of headroom between the two
/// callbacks - that headroom is what absorbs their jitter - so a player
/// hears themselves that far after the fact. The recorder has no such
/// backlog, so without this the sample stored at loop position p is a
/// whole monitoring delay fresher than the one heard at p: a take lands
/// ahead of the beat it was played against, and every layer stacked on
/// top inherits the error again.
///
/// It runs whether or not anything is being recorded, because a take has
/// to open with what was being heard when the loop reached its start
/// point - which is audio from before the take opened.
struct MonitorDelay {
    /// The last `frames` samples, oldest at `at`. Empty means no delay.
    buffer: Vec<f32>,
    at: usize,
}

impl MonitorDelay {
    fn new(frames: usize) -> Self {
        Self {
            buffer: vec![0.0; frames],
            at: 0,
        }
    }

    /// Replaces every sample with the one `frames` earlier, in place.
    fn apply(&mut self, samples: &mut [f32]) {
        if self.buffer.is_empty() {
            return;
        }
        for sample in samples.iter_mut() {
            // The slot holds the oldest sample; swapping retires it and
            // files the new one in its place in a single step.
            std::mem::swap(sample, &mut self.buffer[self.at]);
            self.at = (self.at + 1) % self.buffer.len();
        }
    }
}

/// The input half of the audio path: one interleaved buffer from the
/// driver, reduced to the chosen channel as mono and handed to everyone
/// who wants it.
///
/// A struct rather than a closure body so the path can be driven by a
/// test - a cpal callback needs an open device, which makes anything
/// written inside one unreachable.
pub struct InputPath {
    control: Arc<SharedControl>,
    channels: u16,
    input_channel: u16,
    scratch: Vec<f32>,
    delay: MonitorDelay,
    passthrough: HeapProd<f32>,
    recorder: HeapProd<f32>,
}

impl InputPath {
    pub fn process(&mut self, data: &[f32]) {
        // Bound each chunk to the fixed-size scratch buffer whatever the
        // driver hands us: overrunning it would panic inside a real-time
        // callback. Never happens in practice, but cheap.
        for chunk in data.chunks(SCRATCH_CAPACITY * self.channels as usize) {
            let frames = chunk.len() / self.channels as usize;
            for (i, frame) in chunk.chunks_exact(self.channels as usize).enumerate() {
                self.scratch[i] = frame[self.input_channel as usize];
            }
            let mono = &self.scratch[..frames];

            // A full ring means the far side hasn't drained it, so it is
            // the output that fell behind, not this callback.
            if self.passthrough.push_slice(mono) < mono.len() {
                self.control.note_output_underrun();
            }

            // Held back to match what the passthrough is putting out
            // right now - see `MonitorDelay`.
            let delayed = &mut self.scratch[..frames];
            self.delay.apply(delayed);

            // Through a pre-roll as well: the count-in's take opens on
            // the output side, at the bar line, so the ring has to be
            // flowing by then rather than starting from empty.
            if matches!(
                self.control.load_state(),
                LoopState::Arming | LoopState::Recording | LoopState::Overdubbing
            ) {
                self.recorder.push_slice(delayed);
            }
        }
    }
}

/// The output half: the dry signal, the loop mixed onto it, and the
/// overdub written back. Owns `LoopStack` outright - it lives here and
/// nowhere else, which is why nothing needs a lock to reach it.
pub struct OutputPath {
    control: Arc<SharedControl>,
    channels: u16,
    dry: Vec<f32>,
    loop_out: Vec<f32>,
    recorded: Vec<f32>,
    passthrough: HeapCons<f32>,
    recorder: HeapCons<f32>,
    /// What the stack records, passed on to the UI thread's copy. Fed
    /// from here, after the stack has taken it, so the copy holds exactly
    /// what was recorded - which only this side knows, now that a
    /// count-in's take opens here rather than when the state changes.
    capture: HeapProd<f32>,
    /// Where each take ended, in order, for the same copy: how many of
    /// the samples passed on were that take's, and what became of it.
    /// Only this side knows - the UI thread's state changes reach the
    /// stack a callback later, and some reach it not at all.
    ends: HeapProd<TakeEnd>,
    /// Samples passed on since the last take ended.
    take_samples: usize,
    stack: LoopStack,
    /// REC→DUB with a layer per pass: a dub closes itself once it has
    /// covered the loop, and the next opens on the following sample.
    /// Fixed for the stream's life - a settings change rebuilds it.
    layer_per_pass: bool,
    /// How a first take opens. Count-in and Detect open it here, to the
    /// sample; Instant and Delay are timed on the UI thread, which
    /// publishes Recording itself.
    record_start: RecordStart,
    detector: Detector,
    /// `StopMode::FadeOut`'s length, and how far into it the loop is.
    fade_frames: usize,
    fade_done: usize,
    /// A stop of the audio thread's own has run its course: silent, and
    /// holding its place rather than playing on, until the UI thread's
    /// Stopped arrives - or the stop is called off.
    parked: bool,
    metronome: Metronome,
    /// The callback only ever sees the published state, never the
    /// transitions, so layer bookkeeping keys off this changing - see
    /// `apply_state_change`.
    previous_state: LoopState,
}

impl OutputPath {
    pub fn process(&mut self, data: &mut [f32]) {
        // Same chunk-bounding as the input path.
        for out in data.chunks_mut(SCRATCH_CAPACITY * self.channels as usize) {
            let frames = out.len() / self.channels as usize;
            let dry = &mut self.dry[..frames];

            // A short read means the input hasn't filled the ring yet,
            // so it is the input that fell behind.
            let read = self.passthrough.pop_slice(dry);
            if read < frames {
                dry[read..].fill(0.0);
                self.control.note_input_underrun();
            }

            let state = self.control.load_state();
            // Every block, on or off: a take ending is rounded to the
            // bar at the tempo it was played to.
            self.metronome.set_bpm(self.control.metronome_bpm());
            // After loading the state, before applying it: a state
            // change seen here means the request made before it is seen
            // too, so a double press's take-back - abandoning the layer
            // its first half opened - can't land on the layer before.
            if self.control.take_next_layer_request()
                && self.previous_state == LoopState::Overdubbing
            {
                next_layer(
                    &mut self.stack,
                    &mut self.ends,
                    &self.control,
                    &mut self.take_samples,
                );
            }
            if state != self.previous_state {
                // A count-in may have opened the take already, on the
                // bar line; Recording arriving then is the UI catching up.
                let take_open = self.stack.is_recording();
                let bar = self
                    .control
                    .metronome_on()
                    .then(|| self.metronome.bar_frames());
                let abandon = self.control.take_abandon_take_request();
                let before = self.stack.open_take();
                apply_state_change(
                    &mut self.stack,
                    self.previous_state,
                    state,
                    bar,
                    abandon,
                );
                if let Some(ended) = before
                    && self.stack.open_take().map(|t| t.index) != Some(ended.index)
                {
                    let end = take_end(
                        &self.stack,
                        ended,
                        std::mem::take(&mut self.take_samples),
                        state == LoopState::Idle || abandon,
                    );
                    push_end(&mut self.ends, &self.control, end);
                }
                if state == LoopState::Arming || (state == LoopState::Recording && !take_open) {
                    self.metronome.restart();
                }
                if state == LoopState::Arming {
                    self.detector.reset();
                }
                self.parked = false;
                self.fade_done = 0;
                self.previous_state = state;
            }

            if self.control.take_clear_request() {
                if let Some(open) = self.stack.open_take() {
                    let samples = std::mem::take(&mut self.take_samples);
                    let end = take_end(&self.stack, open, samples, true);
                    push_end(&mut self.ends, &self.control, end);
                }
                self.stack.clear();
                push_end(&mut self.ends, &self.control, TakeEnd::Cleared);
            }
            if self.control.take_remove_layer_request() {
                self.stack.remove_last_layer();
            }
            // After the remove: both on one callback means remove, then
            // bring back, in the order they were asked for.
            if self.control.take_redo_layer_request() {
                self.stack.redo_layer();
            }
            self.stack.set_muted(self.control.muted_layers());
            // Before mixing moves it on: the clicks in this block line
            // up with the loop audio in it.
            let loop_pos = self.stack.play_pos();

            match state {
                LoopState::Recording => {
                    let n = self.recorder.pop_slice(&mut self.recorded[..frames]);
                    self.stack.record_first_layer(&self.recorded[..n]);
                    self.take_samples += pass_on(&mut self.capture, &self.control, &self.recorded[..n]);
                }
                LoopState::Arming => {
                    // Drained whether or not any of it is kept, so it
                    // can't back up while the count runs.
                    let n = self.recorder.pop_slice(&mut self.recorded[..frames]);
                    let keep_from = if self.stack.is_recording() {
                        Some(0)
                    } else {
                        match self.record_start {
                            // The count-in's bar line: the take opens on
                            // it, to the frame, and what came before is
                            // dropped.
                            RecordStart::CountIn => {
                                self.metronome.frames_to_bar_line().filter(|&at| at < frames)
                            }
                            RecordStart::Detect => self.detector.find(&self.recorded[..n]),
                            RecordStart::Instant | RecordStart::Delay => None,
                        }
                    };
                    if let Some(from) = keep_from {
                        if !self.stack.is_recording() {
                            self.stack.begin_first_layer();
                            if self.record_start == RecordStart::Detect {
                                for part in self.detector.before() {
                                    self.stack.record_first_layer(part);
                                    self.take_samples +=
                                        pass_on(&mut self.capture, &self.control, part);
                                }
                            }
                        }
                        let take = &self.recorded[from.min(n)..n];
                        self.stack.record_first_layer(take);
                        self.take_samples += pass_on(&mut self.capture, &self.control, take);
                    }
                }
                LoopState::Looping | LoopState::Overdubbing => {
                    if state == LoopState::Overdubbing {
                        let n = self.recorder.pop_slice(&mut self.recorded[..frames]);
                        // A short read means the input fell behind;
                        // record the gap as silence rather than
                        // shifting everything after it out of time.
                        self.recorded[n..frames].fill(0.0);
                        let gain = self.control.volume_pct();
                        // In pieces where a pass ends mid-block, so the
                        // next layer starts on the very next sample.
                        let mut done = 0;
                        while done < frames {
                            let left = self.layer_per_pass.then(|| self.stack.pass_left()).flatten();
                            if left == Some(0) {
                                next_layer(
                                    &mut self.stack,
                                    &mut self.ends,
                                    &self.control,
                                    &mut self.take_samples,
                                );
                                continue;
                            }
                            let end = left.map_or(frames, |left| frames.min(done + left));
                            let recorded = &self.recorded[done..end];
                            self.stack
                                .read_mixed_with_overdub(&mut self.loop_out[done..end], recorded, gain);
                            self.take_samples += pass_on(&mut self.capture, &self.control, recorded);
                            done = end;
                        }
                    } else {
                        let gain = self.control.volume_pct();
                        let loop_out = &mut self.loop_out[..frames];
                        let pending = self.control.pending_stop();
                        if pending.is_none() {
                            self.parked = false;
                            self.fade_done = 0;
                        }
                        match pending {
                            _ if self.parked => loop_out.fill(0.0),
                            // Up to the top and no further: the next play
                            // starts there.
                            Some(StopMode::LoopEnd) => {
                                let to_end = self.stack.frames_to_loop_end();
                                if to_end <= frames {
                                    self.stack.read_mixed(&mut loop_out[..to_end], gain);
                                    loop_out[to_end..].fill(0.0);
                                    self.parked = true;
                                    self.control.note_stop_done();
                                } else {
                                    self.stack.read_mixed(loop_out, gain);
                                }
                            }
                            Some(StopMode::FadeOut) => {
                                self.stack.read_mixed(loop_out, gain);
                                let total = self.fade_frames as f32;
                                for sample in loop_out.iter_mut() {
                                    *sample *= (total - self.fade_done as f32).max(0.0) / total;
                                    self.fade_done += 1;
                                }
                                if self.fade_done >= self.fade_frames {
                                    self.parked = true;
                                    self.control.note_stop_done();
                                }
                            }
                            Some(StopMode::Immediate) | None => self.stack.read_mixed(loop_out, gain),
                        }
                    }
                    mix_add(dry, &self.loop_out[..frames]);
                }
                LoopState::Idle | LoopState::Stopped => {}
            }

            // Onto the mix, after the loop volume: the click is level
            // with the loop, not scaled by it.
            // A count-in clicks with the metronome off too - a bar of
            // silence would count nobody in.
            let counting_in =
                state == LoopState::Arming && self.record_start == RecordStart::CountIn;
            // Not while parked: the loop holds its place, and the click
            // would sound beat one there every block.
            if (self.control.metronome_on() || counting_in) && !self.parked {
                if matches!(state, LoopState::Looping | LoopState::Overdubbing) {
                    self.metronome
                        .add_on_loop(dry, loop_pos, self.stack.recorded_len());
                } else {
                    self.metronome.add_free(dry);
                }
            }

            duplicate_mono_to_channels(dry, self.channels, out);
        }

        self.control.publish_telemetry(
            self.stack.recorded_len(),
            self.stack.play_pos(),
            self.stack.layer_count(),
            self.stack.is_recording(),
        );
    }
}

/// What the UI thread's copy is told as `ended` is over: `samples` of
/// what was passed on were its, and it's kept unless `dropped` - thrown
/// away by a double press, a cancelled count-in or a clear.
fn take_end(stack: &LoopStack, ended: OpenTake, samples: usize, dropped: bool) -> TakeEnd {
    if dropped {
        TakeEnd::Dropped {
            samples,
            first: ended.first,
        }
    } else if ended.first {
        TakeEnd::First {
            samples,
            loop_len: stack.recorded_len(),
        }
    } else {
        TakeEnd::Overdub {
            samples,
            start: ended.start,
        }
    }
}

/// `RecordStart::Detect`: finds where the input first reaches a level,
/// keeping what came just before. A note's attack starts below the level
/// it's caught at, so the take opens `DETECT_LOOKBACK_MS` back.
struct Detector {
    level: f32,
    /// The latest input, round and round; `filled` counts what's real
    /// since the last reset.
    lookback: Vec<f32>,
    pos: usize,
    filled: usize,
    /// How much of the look-back the take last found opens with.
    wanted: usize,
}

impl Detector {
    fn new(level: f32, lookback: usize) -> Self {
        Self {
            level,
            lookback: vec![0.0; lookback],
            pos: 0,
            filled: 0,
            wanted: 0,
        }
    }

    /// Forgets the look-back, so input from before Arming isn't kept.
    fn reset(&mut self) {
        self.filled = 0;
    }

    /// Where in `block` a take reaching the level opens - the look-back
    /// before the crossing, as far as the block holds it - or `None`,
    /// keeping the block as look-back for the next.
    fn find(&mut self, block: &[f32]) -> Option<usize> {
        let len = self.lookback.len();
        let Some(at) = block.iter().position(|sample| sample.abs() >= self.level) else {
            for &sample in block {
                self.lookback[self.pos] = sample;
                self.pos = (self.pos + 1) % len;
            }
            self.filled = (self.filled + block.len()).min(len);
            return None;
        };
        self.wanted = len.saturating_sub(at).min(self.filled);
        Some(at.saturating_sub(len))
    }

    /// The rest of the look-back the take last found opens with, from
    /// the blocks before its own, oldest first.
    fn before(&self) -> [&[f32]; 2] {
        let len = self.lookback.len();
        let wanted = self.wanted;
        let start = (self.pos + len - wanted) % len;
        if start + wanted <= len {
            [&self.lookback[start..start + wanted], &[]]
        } else {
            [&self.lookback[start..], &self.lookback[..start + wanted - len]]
        }
    }
}

/// Closes the dub in progress and opens the next on the same sample,
/// reporting the end like any other - for REC→DUB's press, and for a
/// layer per pass. Free rather than a method, so it can run while the
/// callback holds its scratch buffers.
fn next_layer(
    stack: &mut LoopStack,
    ends: &mut HeapProd<TakeEnd>,
    control: &SharedControl,
    take_samples: &mut usize,
) {
    let Some(ended) = stack.open_take() else {
        return;
    };
    stack.finish_overdub();
    let end = take_end(stack, ended, std::mem::take(take_samples), false);
    push_end(ends, control, end);
    stack.begin_overdub();
}

/// A full ring is counted as lost capture, like a full sample ring: the
/// copy can no longer tell where takes end, so it mustn't save.
fn push_end(ends: &mut HeapProd<TakeEnd>, control: &SharedControl, end: TakeEnd) {
    if ends.try_push(end).is_err() {
        control.note_capture_lost(1);
    }
}

/// Both halves and the rings between them, built as one piece because
/// that is where the recorded signal's alignment against the monitored
/// signal is decided - see the prefill below.
pub struct AudioPath {
    pub input: InputPath,
    pub output: OutputPath,
}

/// Everything downstream of the device: no cpal types, so a test can
/// build one and push buffers through it.
pub fn build_audio_path(
    control: &Arc<SharedControl>,
    settings: &AppConfig,
    sample_rate: u32,
    input_channels: u16,
    output_channels: u16,
    restored: &[Vec<f32>],
) -> (AudioPath, Capture) {
    // Headroom between the callbacks, to absorb their jitter. It delays
    // the monitored signal by that much as a side effect, which is why
    // the recorded signal is held back to match - see `MonitorDelay`.
    // Everything below is mono.
    let latency_frames = (settings.latency_ms as usize * sample_rate as usize) / 1_000;

    // Room for the delay *and* a whole callback on top of it. The delay
    // is the backlog these rings carry at rest, so a push has to fit
    // above it or it spills and the samples are gone - and a driver
    // buffer is easily larger than the delay (512 frames against 8 ms of
    // headroom at 44.1 kHz is 512 against 352). `SCRATCH_CAPACITY` is
    // already this file's bound on one callback, and at 32768 frames it
    // absorbs many of them.
    let ring_capacity = latency_frames + SCRATCH_CAPACITY;

    // Dry passthrough bridge. The prefill, not the capacity, is what
    // sets the monitoring delay.
    let (mut passthrough_tx, passthrough_rx) = HeapRb::<f32>::new(ring_capacity).split();
    for _ in 0..latency_frames {
        passthrough_tx.try_push(0.0).unwrap();
    }

    // Feeds captured samples to the output path, which owns the layer
    // stack.
    let (recorder_tx, recorder_rx) = HeapRb::<f32>::new(ring_capacity).split();

    // The same samples again, for the copy the UI thread keeps so that
    // the loop can be saved.
    let (capture_tx, capture_rx) =
        HeapRb::<f32>::new(CAPTURE_BUFFER_SECONDS * sample_rate as usize).split();
    // And where each take in them ends. A take ends at most once per
    // state change, a few a second at the very most; this is room for
    // seconds of them.
    let (ends_tx, ends_rx) = HeapRb::<TakeEnd>::new(TAKE_ENDS_CAPACITY).split();

    let loop_capacity = settings.max_loop_secs as usize * sample_rate as usize;
    let mut stack = LoopStack::new(loop_capacity, settings.max_layers as usize);
    for layer in restored {
        let added = stack.add_layer(layer);
        // Anything `loop_mirror::load` handed back fits by construction.
        // Dropping one quietly here would leave the UI thread's copy
        // holding layers that aren't playing.
        debug_assert!(added, "a restored layer should fit the settings it was loaded for");
    }

    let path = AudioPath {
        input: InputPath {
            control: Arc::clone(control),
            channels: input_channels,
            input_channel: settings.input_channel,
            scratch: vec![0.0f32; SCRATCH_CAPACITY],
            delay: MonitorDelay::new(latency_frames),
            passthrough: passthrough_tx,
            recorder: recorder_tx,
        },
        output: OutputPath {
            control: Arc::clone(control),
            channels: output_channels,
            dry: vec![0.0f32; SCRATCH_CAPACITY],
            loop_out: vec![0.0f32; SCRATCH_CAPACITY],
            recorded: vec![0.0f32; SCRATCH_CAPACITY],
            passthrough: passthrough_rx,
            recorder: recorder_rx,
            capture: capture_tx,
            ends: ends_tx,
            take_samples: 0,
            stack,
            layer_per_pass: settings.record_mode == RecordMode::RecDubPerPass,
            record_start: settings.record_start,
            fade_frames: (sample_rate as usize * settings.fade_ms as usize / 1_000).max(1),
            fade_done: 0,
            parked: false,
            detector: Detector::new(
                10f32.powf(settings.detect_db as f32 / 20.0),
                (sample_rate as usize * DETECT_LOOKBACK_MS / 1_000).max(1),
            ),
            metronome: Metronome::new(sample_rate, settings.metronome_bpm),
            previous_state: LoopState::Idle,
        },
    };
    (
        path,
        Capture {
            samples: capture_rx,
            ends: ends_rx,
        },
    )
}

/// The two halves as the backend sees them.
///
/// These impls live here rather than in the app because neither the
/// trait nor the type is the app's: the orphan rule decides where they
/// go, and it is right to - the paths and the promise they keep (no
/// allocation, no locks, bounded work) belong together.
impl InputProcessor for InputPath {
    fn process(&mut self, input: &[f32]) {
        InputPath::process(self, input);
    }
}

impl OutputProcessor for OutputPath {
    fn process(&mut self, output: &mut [f32]) {
        OutputPath::process(self, output);
    }
}

/// A running looper: the streams, and the channel the UI thread reads
/// captured samples from - it keeps its own copy of the loop, since the
/// layer stack itself is out of reach inside the output callback.
pub struct LooperStreams {
    /// Dropping this stops the audio.
    pub stream: Box<dyn AudioStream>,
    pub sample_rate: u32,
    pub captured: Capture,
}

/// One end of the pair `settings` names.
///
/// The direction is part of the lookup, not decoration: an interface's
/// capture and render halves can arrive under the *same* name, so a
/// search by name alone finds whichever the backend happened to list
/// first - and a render endpoint asked to capture records the speakers
/// rather than failing.
pub fn find_device(
    backends: &Backends,
    backend: &str,
    name: &str,
    wanted: Direction,
) -> Result<DeviceInfo, String> {
    let backend = backends
        .by_name(backend)
        .ok_or_else(|| format!("audio backend '{backend}' is not available"))?;
    backend
        .devices()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|device| {
            device.id.name == name
                && match wanted {
                    Direction::Output => device.direction.can_play(),
                    _ => device.direction.can_capture(),
                }
        })
        .ok_or_else(|| format!("audio device '{name}' not found"))
}

/// Both ends. On a duplex backend they are the same device.
pub fn resolve_devices(
    backends: &Backends,
    settings: &AppConfig,
) -> Result<(DeviceInfo, DeviceInfo), String> {
    let input = find_device(
        backends,
        &settings.backend,
        &settings.device_name,
        Direction::Input,
    )?;
    let output = find_device(
        backends,
        &settings.backend,
        settings.output_device(),
        Direction::Output,
    )?;
    Ok((input, output))
}

/// Opens the device named in `settings` and starts the looper on it.
///
/// Only the chosen input channel is captured, treated as mono and
/// duplicated across every output channel; live input always passes
/// through, recording/looping follows `control`. `LoopStack` lives only
/// inside the output path, so nothing here needs a lock. `restored`
/// seeds the layer stack with a loop saved earlier, already held to
/// these settings by `loop_mirror::load`.
pub fn build_looper_streams(
    backends: &Backends,
    control: Arc<SharedControl>,
    settings: &AppConfig,
    restored: &[Vec<f32>],
    on_error: ErrorSink,
) -> Result<LooperStreams, String> {
    let (input, output) = resolve_devices(backends, settings)?;
    let request = StreamRequest {
        input: input.id,
        output: output.id,
        sample_rate: settings.sample_rate,
    };
    let open = backends.open(&request).map_err(|e| e.to_string())?;

    // Read what was granted rather than what was asked for: everything
    // below is sized in samples, and a device that handed back another
    // rate would leave every one of those sizes wrong.
    let format = open.format();
    if settings.input_channel >= format.input_channels {
        return Err(format!(
            "input channel {} is out of range (device has {} channel(s))",
            settings.input_channel + 1,
            format.input_channels
        ));
    }

    let (path, captured) = build_audio_path(
        &control,
        settings,
        format.sample_rate,
        format.input_channels,
        format.output_channels,
        restored,
    );
    let AudioPath { input, output } = path;

    let stream = open
        .start(Box::new(input), Box::new(output), on_error)
        .map_err(|e| e.to_string())?;

    Ok(LooperStreams {
        stream,
        sample_rate: format.sample_rate,
        captured,
    })
}

/// Layer bookkeeping that has to happen exactly when the state changes
/// rather than on every callback: fixing the loop length once the first
/// recording ends, and opening or closing an overdub layer.
///
/// `bar_frames` is set while the metronome is on: a first take played
/// against it comes out a whole number of its bars. `abandon` drops an
/// overdub this change ends, rather than keeping it.
fn apply_state_change(
    stack: &mut LoopStack,
    from: LoopState,
    to: LoopState,
    bar_frames: Option<f64>,
    abandon: bool,
) {
    match from {
        LoopState::Recording => match bar_frames {
            Some(bar) => stack.finish_first_layer_in_bars(bar),
            None => stack.finish_first_layer(),
        },
        LoopState::Overdubbing if abandon => stack.abandon_overdub(),
        LoopState::Overdubbing => stack.finish_overdub(),
        _ => {}
    }
    match to {
        LoopState::Recording if !stack.is_recording() => stack.begin_first_layer(),
        // A count-in called off after its take had opened.
        LoopState::Idle if from == LoopState::Arming && stack.is_recording() => stack.clear(),
        LoopState::Overdubbing => {
            // Where it begins goes to the UI thread's copy with its end.
            stack.begin_overdub();
        }
        _ => {}
    }
}

/// Hands recorded samples to the UI thread's copy of the loop, returning
/// how many went - what a take's end report counts. A full ring is
/// counted rather than waited on: that copy no longer matches what's
/// playing, and saving it would be wrong.
fn pass_on(capture: &mut HeapProd<f32>, control: &SharedControl, samples: &[f32]) -> usize {
    let passed = capture.push_slice(samples);
    if passed < samples.len() {
        control.note_capture_lost(samples.len() - passed);
    }
    passed
}

/// Adds `loop_signal` onto `dry` in place. Not clamped - the sum can run
/// past full scale and is brought back once, at the conversion out to the
/// device, so the volume control can still rescue a hot loop.
fn mix_add(dry: &mut [f32], loop_signal: &[f32]) {
    for (d, l) in dry.iter_mut().zip(loop_signal.iter()) {
        *d += *l;
    }
}

/// Duplicates mono across every output channel, so a single input is
/// centered rather than coming out one side only.
fn duplicate_mono_to_channels(mono: &[f32], channels: u16, out: &mut [f32]) {
    for (frame_out, &sample) in out.chunks_exact_mut(channels as usize).zip(mono.iter()) {
        frame_out.fill(sample);
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
