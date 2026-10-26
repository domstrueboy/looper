use super::*;

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
/// handed the same samples in the first place.
#[test]
fn the_recorder_and_the_mirror_are_fed_the_same_samples() {
    let settings = test_settings();
    let control = Arc::new(SharedControl::new(settings.volume_pct, settings.metronome_bpm));
    let (mut path, mut captured) = build_audio_path(&control, &settings, TEST_RATE, 1, 1, &[]);

    control.publish_state(LoopState::Recording);
    let mut next = 1.0;
    // Only the input half, so nothing drains the recorder ring; two
    // blocks stay well inside it.
    for _ in 0..2 {
        path.input.process(&ramp(&mut next));
    }

    let mut recorded = vec![0.0f32; BLOCK * 2];
    let n = path.output.recorder.pop_slice(&mut recorded);
    let mut mirrored = vec![0.0f32; BLOCK * 2];
    let m = captured.pop_slice(&mut mirrored);

    assert_eq!(n, BLOCK * 2, "both blocks reached the recorder");
    assert_eq!(recorded[..n], mirrored[..m], "the same samples, delayed alike");
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
