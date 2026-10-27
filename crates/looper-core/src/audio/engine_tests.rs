use super::*;
use crate::config::StopMode;

#[test]
fn mix_add_sums_dry_and_loop_signal() {
    let mut dry = [10.0, 20.0, 30.0];
    mix_add(&mut dry, &[1.0, 2.0, 3.0]);
    assert_eq!(dry, [11.0, 22.0, 33.0]);
}

#[test]
fn mix_add_lets_the_sum_run_past_full_scale() {
    // Deliberately not clamped here. A hot loop over a loud dry signal
    // keeps its true value all the way to `sample::to_pcm32`, which is
    // the only place that clips - so turning the volume down still
    // rescues it rather than finding it already squared off.
    let mut dry = [0.9, -0.9];
    mix_add(&mut dry, &[0.8, -0.8]);
    assert_eq!(dry, [1.7, -1.7]);
}

#[test]
fn duplicate_mono_to_channels_fills_every_channel_with_the_same_sample() {
    let mono = [1.0, 2.0, 3.0];
    let mut out = [0.0; 6];
    duplicate_mono_to_channels(&mono, 2, &mut out);
    assert_eq!(out, [1.0, 1.0, 2.0, 2.0, 3.0, 3.0]);
}

#[test]
fn duplicate_mono_to_channels_handles_mono_output() {
    let mono = [5.0, 6.0];
    let mut out = [0.0; 2];
    duplicate_mono_to_channels(&mono, 1, &mut out);
    assert_eq!(out, [5.0, 6.0]);
}

// --- The audio path, driven without a device ---------------------------
//
// Small enough to follow by hand: at 1 kHz an 8 ms latency setting is
// 8 samples of headroom, and a 32-sample loop is eight callbacks of
// four frames. Mono, so a frame is a sample.

const TEST_RATE: u32 = 1_000;
const TEST_LATENCY_MS: u32 = 8;
const BLOCK: usize = 4;
const LOOP_LEN: usize = 32;
const CYCLES_PER_LOOP: usize = LOOP_LEN / BLOCK;

fn test_settings() -> AppConfig {
    AppConfig {
        sample_rate: TEST_RATE,
        latency_ms: TEST_LATENCY_MS,
        // The loop is silenced so the output carries the monitored
        // signal by itself. What gets recorded doesn't depend on gain.
        volume_pct: 0,
        ..AppConfig::default()
    }
}

/// One buffer switch: the input callback, then the output one, the order
/// the driver runs them in. Returns what the monitor put out.
fn drive(path: &mut AudioPath, input: &[f32]) -> Vec<f32> {
    path.input.process(input);
    let mut out = vec![0.0f32; input.len()];
    path.output.process(&mut out);
    out
}

/// The next block of a ramp whose sample values are their own index, so
/// a value read anywhere says which input sample it came from.
///
/// Well outside the +/-1.0 an audio sample keeps to, on purpose: nothing
/// in the path clamps, so the values arrive intact and an offset below
/// reads directly in samples. Clipping happens at the device edge, which
/// driving the path by hand deliberately bypasses.
fn ramp(next: &mut f32) -> Vec<f32> {
    let block: Vec<f32> = (0..BLOCK).map(|i| *next + i as f32).collect();
    *next += BLOCK as f32;
    block
}

/// How far an overdubbed sample is stored from where the player heard
/// it, in samples. Zero is correct: the note heard against loop
/// position p should be the note stored at p.
///
/// The dry path is deliberately delayed by `latency_frames` to absorb
/// jitter, so if the recorder isn't delayed with it, a take lands that
/// far ahead of the beat it was played against - and again per layer.
fn overdub_offset(latency_ms: u32) -> f32 {
    let settings = AppConfig {
        latency_ms,
        ..test_settings()
    };
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, _captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);

    // Silence until the loop exists, so the layer underneath the overdub
    // contributes nothing to what comes back out.
    control.publish_state(LoopState::Idle);
    for _ in 0..CYCLES_PER_LOOP {
        drive(&mut path, &[0.0f32; BLOCK]);
    }
    control.publish_state(LoopState::Recording);
    for _ in 0..CYCLES_PER_LOOP {
        drive(&mut path, &[0.0f32; BLOCK]);
    }

    // A ramp from here on, for long enough to push the silence right
    // out of the monitoring delay before the take starts - whole loops
    // of it, so playback still ends up back at the top.
    let latency_frames = (settings.latency_ms as usize * TEST_RATE as usize) / 1_000;
    let mut next = 1.0;
    control.publish_state(LoopState::Looping);
    for _ in 0..(latency_frames / LOOP_LEN + 2) * CYCLES_PER_LOOP {
        drive(&mut path, &ramp(&mut next));
    }
    assert_eq!(
        path.output.stack.play_pos(),
        0,
        "the loop should be back at the top before the take starts"
    );

    // One full overdub pass, noting where the loop was and what the
    // monitor put out on each callback.
    control.publish_state(LoopState::Overdubbing);
    let mut heard = vec![0.0f32; LOOP_LEN];
    for _ in 0..CYCLES_PER_LOOP {
        let at = path.output.stack.play_pos();
        let out = drive(&mut path, &ramp(&mut next));
        heard[at..at + BLOCK].copy_from_slice(&out);
    }
    assert_eq!(path.output.stack.play_pos(), 0, "one whole pass");

    // The take is still open, so it's the layer being recorded - which
    // `read_mixed` includes over the part already written. The layer
    // below is silent, so this reads back the take itself.
    let mut stored = vec![0.0f32; LOOP_LEN];
    path.output.stack.read_mixed(&mut stored, 100);

    let offsets: Vec<f32> = stored
        .iter()
        .zip(&heard)
        .map(|(stored, heard)| stored - heard)
        .collect();
    assert!(
        offsets.iter().all(|offset| *offset == offsets[0]),
        "expected one constant offset, got {offsets:?}"
    );
    offsets[0]
}

#[test]
fn an_overdub_is_stored_where_it_was_heard() {
    // Latencies at or above the block size; below that the passthrough
    // ring can't hold one callback at all, which is its own problem.
    for latency_ms in [8, 20, 50] {
        let offset = overdub_offset(latency_ms);
        println!("latency {latency_ms} ms: overdub offset {offset}");
        assert_eq!(
            offset, 0.0,
            "at {latency_ms} ms a take lands {offset} samples from the beat it \
             was played against, and every further layer inherits it again"
        );
    }
}

#[test]
fn the_monitor_delay_hands_back_what_went_in_that_many_samples_ago() {
    let mut delay = MonitorDelay::new(3);

    let mut first = [1.0, 2.0, 3.0, 4.0];
    delay.apply(&mut first);
    assert_eq!(first, [0.0, 0.0, 0.0, 1.0], "silence until the line fills");

    let mut second = [5.0, 6.0, 7.0, 8.0];
    delay.apply(&mut second);
    assert_eq!(second, [2.0, 3.0, 4.0, 5.0]);
}

#[test]
fn no_delay_leaves_the_signal_alone() {
    let mut delay = MonitorDelay::new(0);
    let mut samples = [1.0, 2.0, 3.0];
    delay.apply(&mut samples);
    assert_eq!(samples, [1.0, 2.0, 3.0], "a zero-latency setting is not possible, but still");
}

/// The layer stack and the UI thread's copy are laid out by two separate
/// pieces of code that have to agree, which they can only do if they are
/// handed the same samples in the first place. The copy is fed what the
/// stack took, after it took it.
#[test]
fn the_mirror_is_fed_exactly_what_the_stack_records() {
    let settings = test_settings();
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, mut captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);

    control.publish_state(LoopState::Recording);
    let mut next = 1.0;
    for _ in 0..4 {
        drive(&mut path, &ramp(&mut next));
    }

    let mut mirrored = vec![0.0f32; BLOCK * 8];
    let m = captured.samples.pop_slice(&mut mirrored);
    assert_eq!(m, path.output.stack.recorded_len(), "all of it, and no more");
    // Held back by the 8-sample monitoring delay, like the take itself.
    let expected: Vec<f32> = (0..m)
        .map(|i| if i < 8 { 0.0 } else { (i - 7) as f32 })
        .collect();
    assert_eq!(mirrored[..m], expected);
}

/// A count-in opens the take on the bar line to the sample - here
/// mid-block, where the UI thread's frame clock would have landed it
/// anywhere. With the metronome off: the count-in clicks regardless.
#[test]
fn a_count_in_opens_the_take_on_the_bar_line() {
    // 130 BPM at 1 kHz: a bar of 1,846.15 frames, so the line falls at
    // frame 1,846 - half way through a four-frame block.
    let settings = AppConfig {
        metronome_bpm: 130,
        record_start: RecordStart::CountIn,
        ..test_settings()
    };
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, mut captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);

    control.publish_state(LoopState::Arming);
    let mut next = 1.0;
    for _ in 0..(1_846 + 14) / BLOCK {
        drive(&mut path, &ramp(&mut next));
    }

    assert_eq!(path.output.stack.recorded_len(), 14, "from the line on");
    let mut first = [0.0f32; 1];
    captured.samples.pop_slice(&mut first);
    // Input sample 1,838 (valued 1,839), heard at frame 1,846 through
    // the 8-sample monitoring delay.
    assert_eq!(first[0], 1_839.0);
}

#[test]
fn a_delay_records_nothing_while_it_runs() {
    // Timed on the UI thread, which publishes Recording itself when it
    // runs out - the metronome being on changes nothing.
    let settings = test_settings();
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, _captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);

    control.publish_state(LoopState::Arming);
    let mut next = 1.0;
    for _ in 0..1_000 {
        drive(&mut path, &ramp(&mut next));
    }
    assert_eq!(path.output.stack.recorded_len(), 0);
}

/// A driver hands over whole buffers, and a buffer is routinely larger
/// than the monitoring delay - 512 frames against 8 ms of headroom at
/// 44.1 kHz. The ring has to hold the delay and a callback on top of it,
/// or every push spills and the monitored signal is shredded.
#[test]
fn a_callback_larger_than_the_monitoring_delay_is_not_dropped() {
    let settings = test_settings();
    let latency_frames = (settings.latency_ms as usize * TEST_RATE as usize) / 1_000;
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, _captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);

    let frames = latency_frames * 4;
    let input: Vec<f32> = (1..=frames).map(|i| i as f32).collect();
    path.input.process(&input);

    let (_, spilled) = control.take_underrun_counts();
    assert_eq!(spilled, 0, "the passthrough ring could not hold one callback");

    let mut out = vec![0.0f32; frames];
    path.output.process(&mut out);

    let mut expected = vec![0.0f32; latency_frames];
    expected.extend_from_slice(&input[..frames - latency_frames]);
    assert_eq!(out, expected, "delayed by the monitoring delay, otherwise whole");
}

/// The two ends need not carry the same number of channels - a capture
/// endpoint is often mono where the render one is stereo. Feeding both
/// paths one count would de-interleave at the wrong stride, which fails
/// silently: no error, just the wrong samples in the wrong slots.
#[test]
fn the_input_and_output_channel_counts_are_independent() {
    let settings = test_settings();
    let latency_frames = (settings.latency_ms as usize * TEST_RATE as usize) / 1_000;
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, _captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 2, &[]);

    // One mono channel in, two out, and long enough to push the
    // monitoring delay through so the output carries real samples.
    let frames = latency_frames + BLOCK;
    let input: Vec<f32> = (1..=frames).map(|i| i as f32).collect();
    path.input.process(&input);

    let mut out = vec![0.0f32; frames * 2];
    path.output.process(&mut out);

    let first = latency_frames * 2;
    assert_eq!(out[first], input[0], "delayed by the monitoring delay");
    assert_eq!(out[first], out[first + 1], "and centred across both channels");
}

#[test]
fn a_count_in_called_off_after_its_take_opened_leaves_no_loop() {
    // The take opens here on the bar line; a cancel published before the
    // UI thread saw it arrives as Arming -> Idle with the take open.
    let settings = AppConfig {
        record_start: RecordStart::CountIn,
        ..test_settings()
    };
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, _captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);
    control.publish_state(LoopState::Arming);
    let mut next = 1.0;
    for _ in 0..(2_000 + 40) / BLOCK {
        drive(&mut path, &ramp(&mut next));
    }
    assert!(path.output.stack.recorded_len() > 0, "opened");

    control.publish_state(LoopState::Idle);
    drive(&mut path, &ramp(&mut next));

    assert_eq!(path.output.stack.recorded_len(), 0);
    assert!(!path.output.stack.is_recording());
}

/// Every end the audio thread has reported so far.
fn ends(captured: &mut Capture) -> Vec<TakeEnd> {
    let mut ends = Vec::new();
    while let Some(end) = captured.ends.try_pop() {
        ends.push(end);
    }
    ends
}

#[test]
fn a_take_ends_with_a_count_of_exactly_its_samples() {
    // What the UI thread's copy splits its stream by - so it has to be
    // what was passed on, to the sample, not what the state change said.
    let settings = test_settings();
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, mut captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);
    let mut next = 1.0;

    control.publish_state(LoopState::Recording);
    for _ in 0..CYCLES_PER_LOOP {
        drive(&mut path, &ramp(&mut next));
    }
    control.publish_state(LoopState::Looping);
    drive(&mut path, &ramp(&mut next));

    assert_eq!(
        ends(&mut captured),
        vec![TakeEnd::First {
            samples: LOOP_LEN,
            loop_len: LOOP_LEN,
        }]
    );
}

#[test]
fn recording_into_a_dub_reports_two_takes_split_where_the_stack_split_them() {
    let settings = test_settings();
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, mut captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);
    let mut next = 1.0;

    control.publish_state(LoopState::Recording);
    for _ in 0..CYCLES_PER_LOOP {
        drive(&mut path, &ramp(&mut next));
    }
    control.publish_state(LoopState::Overdubbing);
    for _ in 0..3 {
        drive(&mut path, &ramp(&mut next));
    }
    control.publish_state(LoopState::Looping);
    drive(&mut path, &ramp(&mut next));

    assert_eq!(
        ends(&mut captured),
        vec![
            TakeEnd::First {
                samples: LOOP_LEN,
                loop_len: LOOP_LEN,
            },
            TakeEnd::Overdub {
                samples: 3 * BLOCK,
                start: 0,
            },
        ]
    );
}

#[test]
fn an_abandoned_dub_is_reported_dropped() {
    let settings = test_settings();
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, mut captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);
    let mut next = 1.0;

    control.publish_state(LoopState::Recording);
    for _ in 0..CYCLES_PER_LOOP {
        drive(&mut path, &ramp(&mut next));
    }
    control.publish_state(LoopState::Looping);
    drive(&mut path, &ramp(&mut next));
    control.publish_state(LoopState::Overdubbing);
    drive(&mut path, &ramp(&mut next));
    control.request_abandon_take();
    control.publish_state(LoopState::Stopped);
    drive(&mut path, &ramp(&mut next));

    let ends = ends(&mut captured);
    assert_eq!(
        ends.last(),
        Some(&TakeEnd::Dropped {
            samples: BLOCK,
            first: false,
        })
    );
    assert_eq!(path.output.stack.layer_count(), 1, "and gone from the stack");
}

/// A first take played round once, then a dub of one block.
fn dubbing(control: &Arc<SharedControl>, path: &mut AudioPath, next: &mut f32) {
    control.publish_state(LoopState::Recording);
    for _ in 0..CYCLES_PER_LOOP {
        drive(path, &ramp(next));
    }
    control.publish_state(LoopState::Looping);
    drive(path, &ramp(next));
    control.publish_state(LoopState::Overdubbing);
    drive(path, &ramp(next));
}

#[test]
fn a_next_layer_request_splits_the_dub_where_it_lands() {
    let settings = test_settings();
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, mut captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);
    let mut next = 1.0;
    dubbing(&control, &mut path, &mut next);

    control.request_next_layer();
    drive(&mut path, &ramp(&mut next));
    control.publish_state(LoopState::Looping);
    drive(&mut path, &ramp(&mut next));

    // The dub opened a block in, after one played; the next layer picks
    // up on the sample the first left off.
    let ends = ends(&mut captured);
    assert_eq!(
        ends[ends.len() - 2..],
        [
            TakeEnd::Overdub {
                samples: BLOCK,
                start: BLOCK,
            },
            TakeEnd::Overdub {
                samples: BLOCK,
                start: 2 * BLOCK,
            },
        ]
    );
    assert_eq!(path.output.stack.layer_count(), 3);
}

#[test]
fn a_take_back_arriving_with_its_next_layer_drops_only_the_new_one() {
    // A double press in REC→DUB, both halves inside one block: the split
    // has to happen before the abandon, or it'd drop the layer before.
    let settings = test_settings();
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, mut captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);
    let mut next = 1.0;
    dubbing(&control, &mut path, &mut next);

    control.request_next_layer();
    control.request_abandon_take();
    control.publish_state(LoopState::Looping);
    drive(&mut path, &ramp(&mut next));

    let ends = ends(&mut captured);
    assert_eq!(
        ends.last(),
        Some(&TakeEnd::Dropped {
            samples: 0,
            first: false,
        })
    );
    assert_eq!(path.output.stack.layer_count(), 2, "the dub before it kept");
}

#[test]
fn a_clear_reports_the_open_take_dropped_then_cleared() {
    let settings = test_settings();
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, mut captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);
    let mut next = 1.0;

    control.publish_state(LoopState::Recording);
    for _ in 0..2 {
        drive(&mut path, &ramp(&mut next));
    }
    control.request_clear();
    control.publish_state(LoopState::Idle);
    drive(&mut path, &ramp(&mut next));

    assert_eq!(
        ends(&mut captured),
        vec![
            TakeEnd::Dropped {
                samples: 2 * BLOCK,
                first: true,
            },
            TakeEnd::Cleared,
        ]
    );
}

// --- A layer per pass -------------------------------------------------------

/// A per-pass path dubbing, the dub opened two samples off the block
/// grid so its pass ends mid-block.
fn dubbing_per_pass(max_layers: u32) -> (Arc<SharedControl>, AudioPath, Capture, f32) {
    let settings = AppConfig {
        record_mode: RecordMode::RecDubPerPass,
        max_layers,
        ..test_settings()
    };
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);
    let mut next = 1.0;
    control.publish_state(LoopState::Recording);
    for _ in 0..CYCLES_PER_LOOP {
        drive(&mut path, &ramp(&mut next));
    }
    control.publish_state(LoopState::Looping);
    drive(&mut path, &ramp(&mut next)[..2]);
    control.publish_state(LoopState::Overdubbing);
    (control, path, captured, next)
}

#[test]
fn a_layer_per_pass_splits_on_the_sample_the_pass_ends() {
    let (control, mut path, mut captured, mut next) = dubbing_per_pass(4);

    // One and a half passes, in whole blocks.
    for _ in 0..CYCLES_PER_LOOP * 3 / 2 {
        drive(&mut path, &ramp(&mut next));
    }
    control.publish_state(LoopState::Looping);
    drive(&mut path, &ramp(&mut next));

    let ends = ends(&mut captured);
    assert_eq!(
        ends[ends.len() - 2..],
        [
            TakeEnd::Overdub {
                samples: LOOP_LEN,
                start: 2,
            },
            TakeEnd::Overdub {
                samples: LOOP_LEN / 2,
                start: 2,
            },
        ],
        "a whole pass, then the half after it, from the same place"
    );
    assert_eq!(path.output.stack.layer_count(), 3);
}

#[test]
fn with_every_layer_used_the_last_keeps_taking_passes() {
    let (control, mut path, mut captured, mut next) = dubbing_per_pass(2);

    for _ in 0..CYCLES_PER_LOOP * 2 {
        drive(&mut path, &ramp(&mut next));
    }
    control.publish_state(LoopState::Looping);
    drive(&mut path, &ramp(&mut next));

    let ends = ends(&mut captured);
    assert_eq!(
        ends.last(),
        Some(&TakeEnd::Overdub {
            samples: 2 * LOOP_LEN,
            start: 2,
        }),
        "one layer, two passes summed into it"
    );
    assert_eq!(path.output.stack.layer_count(), 2);
}

#[test]
fn without_a_layer_per_pass_a_dub_runs_on_into_the_next_pass() {
    let settings = test_settings();
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, _captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);
    let mut next = 1.0;
    dubbing(&control, &mut path, &mut next);

    for _ in 0..CYCLES_PER_LOOP * 2 {
        drive(&mut path, &ramp(&mut next));
    }
    control.publish_state(LoopState::Looping);
    drive(&mut path, &ramp(&mut next));

    assert_eq!(path.output.stack.layer_count(), 2, "one layer, however many passes");
}

// --- Detect -----------------------------------------------------------------

/// A path listening for the first note at -20 dB - 0.1 - with the 10 ms
/// look-back that is 10 samples at 1 kHz.
fn detecting() -> (Arc<SharedControl>, AudioPath, Capture) {
    let settings = AppConfig {
        record_start: RecordStart::Detect,
        detect_db: -20,
        ..test_settings()
    };
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (path, captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);
    control.publish_state(LoopState::Arming);
    (control, path, captured)
}

/// Quiet input, every sample telling which it was: 0.001 x its number.
fn quiet(from: usize, len: usize) -> Vec<f32> {
    (from..from + len).map(|k| k as f32 * 0.001).collect()
}

#[test]
fn detect_opens_the_take_just_before_the_first_note() {
    // The note lands on a block's first sample, so the whole look-back
    // comes from the blocks before - round the end of its buffer.
    let (_control, mut path, mut captured) = detecting();
    for block in 0..20 {
        drive(&mut path, &quiet(1 + block * BLOCK, BLOCK));
    }
    assert_eq!(path.output.stack.recorded_len(), 0, "nothing loud enough yet");
    for _ in 0..5 {
        drive(&mut path, &[0.5; BLOCK]);
    }

    // The monitoring delay holds quiet and loud back alike, so the take
    // opens on the last ten quiet samples played: 71..=80.
    let mut first = [0.0f32; 11];
    captured.samples.pop_slice(&mut first);
    let mut expected = quiet(71, 10);
    expected.push(0.5);
    assert_eq!(first[..], expected[..]);
}

#[test]
fn a_note_deep_in_a_block_takes_its_look_back_from_the_block() {
    let (_control, mut path, mut captured) = detecting();
    let mut block = quiet(1, 24);
    block[20..].fill(0.5);
    drive(&mut path, &block);
    drive(&mut path, &[0.5; 24]);

    // Found 20 samples into what the recorder read: the ten before it
    // are in the same block.
    let mut first = [0.0f32; 11];
    captured.samples.pop_slice(&mut first);
    let mut expected = quiet(11, 10);
    expected.push(0.5);
    assert_eq!(first[..], expected[..]);
}

// --- Stop mode --------------------------------------------------------------

/// A loop of `LOOP_LEN` playing loud enough to hear, two blocks in.
fn playing(settings: AppConfig) -> (Arc<SharedControl>, AudioPath) {
    let settings = AppConfig {
        volume_pct: 100,
        ..settings
    };
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, _captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);
    let mut next = 1.0;
    control.publish_state(LoopState::Recording);
    for _ in 0..CYCLES_PER_LOOP {
        drive(&mut path, &ramp(&mut next));
    }
    control.publish_state(LoopState::Looping);
    for _ in 0..2 {
        drive(&mut path, &[0.0; BLOCK]);
    }
    (control, path)
}

#[test]
fn a_stop_at_the_loop_end_plays_to_the_top_and_holds_there() {
    let (control, mut path) = playing(test_settings());
    control.request_stop(StopMode::LoopEnd);

    for _ in 0..CYCLES_PER_LOOP - 3 {
        drive(&mut path, &[0.0; BLOCK]);
    }
    assert!(!control.take_stop_done(), "a block short of the end");
    drive(&mut path, &[0.0; BLOCK]);
    assert!(control.take_stop_done(), "the end");

    let out = drive(&mut path, &[0.0; BLOCK]);
    assert!(out.iter().all(|&s| s == 0.0), "silent");
    assert_eq!(path.output.stack.play_pos(), 0, "held at the top for the next play");
}

#[test]
fn a_fade_out_takes_its_length_then_holds() {
    // 16 ms at 1 kHz: four blocks, each sample a sixteenth quieter than
    // the same loop left playing.
    let settings = AppConfig {
        fade_ms: 16,
        ..test_settings()
    };
    let (control, mut path) = playing(settings.clone());
    let (_, mut unfaded) = playing(settings);
    control.request_stop(StopMode::FadeOut);

    let mut gains = Vec::new();
    for block in 0..4 {
        assert!(!control.take_stop_done(), "block {block}");
        let faded = drive(&mut path, &[0.0; BLOCK]);
        let full = drive(&mut unfaded, &[0.0; BLOCK]);
        gains.extend(faded.iter().zip(&full).map(|(f, u)| f / u));
    }
    assert!(control.take_stop_done());
    let expected: Vec<f32> = (0..16).map(|i| (16 - i) as f32 / 16.0).collect();
    assert_eq!(gains, expected);

    let held = path.output.stack.play_pos();
    let out = drive(&mut path, &[0.0; BLOCK]);
    assert!(out.iter().all(|&s| s == 0.0));
    assert_eq!(path.output.stack.play_pos(), held);
}

#[test]
fn a_called_off_stop_plays_on_from_where_it_held() {
    let (control, mut path) = playing(test_settings());
    control.request_stop(StopMode::LoopEnd);
    for _ in 0..CYCLES_PER_LOOP {
        drive(&mut path, &[0.0; BLOCK]);
    }

    control.cancel_stop();
    drive(&mut path, &[0.0; BLOCK]);

    assert_eq!(path.output.stack.play_pos(), BLOCK, "moving again, from the top");
}

// --- Levels -----------------------------------------------------------------

/// The first blocks of the click on its own, at `click_pct`.
fn clicking_at(click_pct: u32) -> Vec<f32> {
    let settings = test_settings();
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, _captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);
    control.publish_metronome_on(true);
    control.publish_click_volume_pct(click_pct);
    (0..8).flat_map(|_| drive(&mut path, &[0.0; BLOCK])).collect()
}

#[test]
fn the_click_plays_at_its_level() {
    let full = clicking_at(100);
    assert!(full.iter().any(|&s| s != 0.0), "a click to compare");

    let doubled: Vec<f32> = full.iter().map(|s| s * 2.0).collect();
    assert_eq!(clicking_at(200), doubled);
    assert!(clicking_at(0).iter().all(|&s| s == 0.0));
}

