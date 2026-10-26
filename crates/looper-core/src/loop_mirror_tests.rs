use super::*;

use ringbuf::traits::{Producer, Split};
use ringbuf::{HeapProd, HeapRb};

use crate::audio::loop_stack::LoopStack;
use crate::config::AppConfig;

const RATE: u32 = 44_100;

/// A loop is measured in seconds against `max_loop_secs`, so the tests
/// about that limit run at 1 Hz: three samples is a three-second loop,
/// which keeps them readable instead of megabyte-sized.
const SLOW: u32 = 1;

fn new_mirror() -> (HeapProd<f32>, LoopMirror) {
    new_mirror_at(RATE)
}

fn new_mirror_at(sample_rate: u32) -> (HeapProd<f32>, LoopMirror) {
    let (producer, consumer) = HeapRb::<f32>::new(1024).split();
    (producer, LoopMirror::new(consumer, sample_rate, Vec::new()))
}

/// What a saved loop is checked against on the way back in. Limits are
/// the defaults unless a test is specifically about one of them.
fn settings(sample_rate: u32) -> AppConfig {
    AppConfig {
        sample_rate,
        ..AppConfig::default()
    }
}

/// One take, driven the way a frame loop would: the state changes, the
/// input callback delivers samples, the state changes back. The extra
/// tick is the frame on which the loop length settles.
fn record(
    input: &mut HeapProd<f32>,
    mirror: &mut LoopMirror,
    state: LoopState,
    samples: &[f32],
    loop_len: usize,
    take_start: usize,
) -> bool {
    mirror.tick(state, loop_len, take_start);
    input.push_slice(samples);
    mirror.tick(state, loop_len, take_start);
    mirror.tick(LoopState::Looping, loop_len, take_start);
    mirror.tick(LoopState::Looping, loop_len, take_start)
}

/// Sample values, as small exact fractions.
///
/// A saved loop is written as 32-bit integers, so a value has to survive
/// f32 -> i32 -> f32 unchanged, and has to sit inside +/-1.0 or saving
/// would clip it. Sixty-fourths do both, and keep the tests reading in
/// whole numbers rather than in decimals nobody can check by eye.
fn s(values: &[i32]) -> Vec<f32> {
    values.iter().map(|&v| v as f32 / 64.0).collect()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("looper-pedal-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn a_first_take_becomes_the_only_layer() {
    let (mut input, mut mirror) = new_mirror();

    let out_of_date = record(
        &mut input,
        &mut mirror,
        LoopState::Recording,
        &s(&[1, 2, 3, 4]),
        4,
        0,
    );

    assert!(out_of_date, "a new take needs saving");
    assert_eq!(mirror.layers().len(), 1);
    assert_eq!(mirror.layers(), vec![s(&[1, 2, 3, 4])]);
}

#[test]
fn an_overdub_lands_where_it_began() {
    let (mut input, mut mirror) = new_mirror();
    record(
        &mut input,
        &mut mirror,
        LoopState::Recording,
        &s(&[1, 2, 3, 4]),
        4,
        0,
    );

    // Started two samples into the loop, and only two long.
    record(
        &mut input,
        &mut mirror,
        LoopState::Overdubbing,
        &s(&[9, 8]),
        4,
        2,
    );

    assert_eq!(mirror.layers().len(), 2);
    assert_eq!(mirror.layers()[1], s(&[0, 0, 9, 8]));
}

#[test]
fn a_second_pass_sums_into_the_same_layer() {
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &[0.0; 4], 4, 0);

    // Six samples over a four-sample loop: the last two come back round.
    record(
        &mut input,
        &mut mirror,
        LoopState::Overdubbing,
        &s(&[1, 1, 1, 1, 2, 2]),
        4,
        0,
    );

    assert_eq!(mirror.layers()[1], s(&[3, 3, 1, 1]));
}

#[test]
fn a_laid_out_take_matches_what_the_layer_stack_plays() {
    // The audio thread writes overdubs sample by sample as it plays;
    // this lays the same take out in one go. The two rules are written
    // separately, so this is what keeps them honest.
    let base = s(&[10, 20, 30, 40]);
    let take = s(&[1, 2, 3, 4, 5, 6]);
    let start = 2;

    let mut stack = LoopStack::new(16, 4);
    stack.begin_first_layer();
    stack.record_first_layer(&base);
    stack.finish_first_layer();

    let mut skipped = vec![0.0; start];
    stack.read_mixed(&mut skipped, 100);
    stack.begin_overdub();
    let mut heard = vec![0.0; take.len()];
    stack.read_mixed_with_overdub(&mut heard, &take, 100);
    stack.finish_overdub();

    // The overdub ended back at the top of the loop.
    let mut played = vec![0.0; base.len()];
    stack.read_mixed(&mut played, 100);

    let (mut input, mut mirror) = new_mirror();
    record(
        &mut input,
        &mut mirror,
        LoopState::Recording,
        &base,
        base.len(),
        0,
    );
    record(
        &mut input,
        &mut mirror,
        LoopState::Overdubbing,
        &take,
        base.len(),
        start,
    );

    let mirrored = &mirror.layers()[1];
    let summed: Vec<f32> = base
        .iter()
        .zip(mirrored)
        .map(|(base, layer)| base + layer)
        .collect();
    assert_eq!(summed, played, "mirrored layer should sum to what plays");
}

#[test]
fn recording_again_replaces_the_loop() {
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2]), 2, 0);
    record(
        &mut input,
        &mut mirror,
        LoopState::Overdubbing,
        &s(&[3, 4]),
        2,
        0,
    );
    assert_eq!(mirror.layers().len(), 2);

    record(
        &mut input,
        &mut mirror,
        LoopState::Recording,
        &s(&[5, 6, 7]),
        3,
        0,
    );

    assert_eq!(mirror.layers().len(), 1);
    assert_eq!(mirror.layers(), vec![s(&[5, 6, 7])]);
}

#[test]
fn removing_and_clearing_follow_the_stack() {
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2]), 2, 0);
    record(
        &mut input,
        &mut mirror,
        LoopState::Overdubbing,
        &s(&[3, 4]),
        2,
        0,
    );

    mirror.remove_last_layer();
    assert_eq!(mirror.layers().len(), 1);

    mirror.clear();
    assert_eq!(mirror.layers().len(), 0);
    assert!(mirror.layers().is_empty());
}

#[test]
fn samples_arriving_with_no_take_open_are_dropped() {
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2]), 2, 0);

    // Stragglers from the take that just ended.
    input.push_slice(&s(&[99, 99]));
    mirror.tick(LoopState::Looping, 2, 0);

    record(
        &mut input,
        &mut mirror,
        LoopState::Overdubbing,
        &s(&[3, 4]),
        2,
        0,
    );
    assert_eq!(mirror.layers()[1], s(&[3, 4]), "no stragglers mixed in");
}

#[test]
fn a_saved_loop_comes_back_layer_for_layer() {
    let directory = temp_dir("round-trip");
    let (mut input, mut mirror) = new_mirror();
    record(
        &mut input,
        &mut mirror,
        LoopState::Recording,
        &s(&[1, 2, 3]),
        3,
        0,
    );
    record(
        &mut input,
        &mut mirror,
        LoopState::Overdubbing,
        &s(&[4, 5, 6]),
        3,
        0,
    );

    mirror.save(&directory).expect("saves");
    let loaded = load(&directory, &settings(RATE));

    assert_eq!(loaded, vec![s(&[1, 2, 3]), s(&[4, 5, 6])]);

    delete(&directory);
    assert!(load(&directory, &settings(RATE)).is_empty(), "deleted");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_loop_recorded_at_another_rate_is_not_loaded() {
    let directory = temp_dir("wrong-rate");
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2]), 2, 0);
    mirror.save(&directory).expect("saves");

    // No resampling, so a rate change means the loop is unusable.
    assert!(load(&directory, &settings(48_000)).is_empty());
    assert_eq!(load(&directory, &settings(RATE)).len(), 1);

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_gap_in_what_arrived_stops_the_loop_being_saved() {
    let directory = temp_dir("lost");
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2]), 2, 0);

    mirror.note_lost_samples();

    // Better no save than one that doesn't match what was played.
    assert!(mirror.save(&directory).is_err());
    assert!(load(&directory, &settings(RATE)).is_empty());
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_fresh_loop_can_be_saved_again_after_a_gap() {
    let directory = temp_dir("lost-then-cleared");
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2]), 2, 0);
    mirror.note_lost_samples();
    assert!(mirror.save(&directory).is_err());

    // Clearing throws away the takes the gap applied to, so there is
    // nothing left for it to be right about. Latching it for the whole
    // session would quietly stop saving anything ever again.
    mirror.clear();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[3, 4]), 2, 0);
    mirror.save(&directory).expect("a loop recorded since the gap saves");
    assert_eq!(load(&directory, &settings(RATE)), vec![s(&[3, 4])]);

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn recording_over_a_lost_loop_makes_it_savable_again() {
    let directory = temp_dir("lost-then-rerecorded");
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2]), 2, 0);
    mirror.note_lost_samples();

    // No clear in between: a first take replaces the whole loop by
    // itself, so the gap no longer applies to anything held.
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[5, 6, 7]), 3, 0);
    mirror.save(&directory).expect("saves");
    assert_eq!(load(&directory, &settings(RATE)), vec![s(&[5, 6, 7])]);

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_loop_longer_than_the_setting_now_allows_is_not_loaded() {
    let directory = temp_dir("too-long");
    let (mut input, mut mirror) = new_mirror_at(SLOW);
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2, 3]), 3, 0);
    mirror.save(&directory).expect("saves");

    let mut tightened = settings(SLOW);
    tightened.max_loop_secs = 2;
    assert!(
        load(&directory, &tightened).is_empty(),
        "a three-second loop does not fit a two-second limit"
    );

    tightened.max_loop_secs = 3;
    assert_eq!(load(&directory, &tightened).len(), 1, "three fits three");

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn layers_past_the_setting_are_dropped_and_the_rest_still_load() {
    let directory = temp_dir("too-many");
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2]), 2, 0);
    record(&mut input, &mut mirror, LoopState::Overdubbing, &s(&[3, 4]), 2, 0);
    record(&mut input, &mut mirror, LoopState::Overdubbing, &s(&[5, 6]), 2, 0);
    mirror.save(&directory).expect("saves");

    let mut fewer = settings(RATE);
    fewer.max_layers = 2;
    // Layers are independent takes, so the ones underneath are still
    // exactly what was played - only what no longer fits goes.
    assert_eq!(
        load(&directory, &fewer),
        vec![s(&[1, 2]), s(&[3, 4])],
        "the layers that still fit come back"
    );

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_first_take_that_captured_nothing_clears_the_saved_loop() {
    let directory = temp_dir("empty-retake");
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2]), 2, 0);
    mirror.save(&directory).expect("saves");
    assert_eq!(load(&directory, &settings(RATE)).len(), 1);

    // Record pressed and undone before a single sample arrived.
    mirror.tick(LoopState::Recording, 2, 0);
    let out_of_date = mirror.tick(LoopState::Looping, 0, 0);

    assert!(out_of_date, "the loop is gone, so the saved copy is stale");
    mirror.save(&directory).expect("saves");
    assert!(
        load(&directory, &settings(RATE)).is_empty(),
        "otherwise the old loop comes back at the next launch"
    );

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn redo_puts_the_removed_take_back_on_top() {
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2]), 2, 0);
    record(&mut input, &mut mirror, LoopState::Overdubbing, &s(&[3, 4]), 2, 0);

    mirror.remove_last_layer();
    assert!(mirror.can_redo());
    mirror.redo_layer();

    assert!(!mirror.can_redo());
    assert_eq!(mirror.layers(), vec![s(&[1, 2]), s(&[3, 4])]);
}

#[test]
fn a_new_take_forgets_what_redo_could_bring_back() {
    // The layer stack forgets at the same moment; the two must agree.
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2]), 2, 0);
    record(&mut input, &mut mirror, LoopState::Overdubbing, &s(&[3, 4]), 2, 0);
    mirror.remove_last_layer();

    record(&mut input, &mut mirror, LoopState::Overdubbing, &s(&[5, 6]), 2, 0);

    assert!(!mirror.can_redo());
}

#[test]
fn a_first_take_longer_than_the_loop_is_cut_not_wrapped() {
    // A take rounded down to the bar: the audio thread cut the excess,
    // so summing it back over the top would save something nobody heard.
    let (mut input, mut mirror) = new_mirror();
    record(
        &mut input,
        &mut mirror,
        LoopState::Recording,
        &s(&[1, 2, 3, 4, 5]),
        3,
        0,
    );
    assert_eq!(mirror.layers(), vec![s(&[1, 2, 3])]);
}

#[test]
fn a_first_take_shorter_than_the_loop_ends_in_silence() {
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2]), 4, 0);
    assert_eq!(mirror.layers(), vec![s(&[1, 2, 0, 0])]);
}

#[test]
fn a_count_in_called_off_keeps_nothing_it_captured() {
    let (mut input, mut mirror) = new_mirror();
    mirror.tick(LoopState::Arming, 0, 0);
    input.push_slice(&s(&[1, 2]));
    mirror.tick(LoopState::Arming, 0, 0);

    let out_of_date = mirror.tick(LoopState::Idle, 0, 0);

    assert!(!out_of_date, "nothing to save");
    assert!(mirror.layers().is_empty());
}

#[test]
fn a_count_in_take_becomes_the_loop_once_recording() {
    // Its samples arrive before the state here says Recording.
    let (mut input, mut mirror) = new_mirror();
    mirror.tick(LoopState::Arming, 0, 0);
    input.push_slice(&s(&[1, 2]));
    mirror.tick(LoopState::Arming, 0, 0);
    mirror.tick(LoopState::Recording, 0, 0);
    input.push_slice(&s(&[3, 4]));
    mirror.tick(LoopState::Looping, 4, 0);
    mirror.tick(LoopState::Looping, 4, 0);

    assert_eq!(mirror.layers(), vec![s(&[1, 2, 3, 4])]);
}

#[test]
fn a_count_in_called_off_leaves_redo_alone() {
    // Nothing was replaced, so what redo could bring back still can.
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2]), 2, 0);
    mirror.remove_last_layer();
    mirror.tick(LoopState::Idle, 0, 0);

    mirror.tick(LoopState::Arming, 0, 0);
    mirror.tick(LoopState::Idle, 0, 0);

    assert!(mirror.can_redo());
}

// --- The saved tempo --------------------------------------------------------

#[test]
fn the_beat_count_reads_back() {
    let dir = temp_dir("mirror-beats");
    save_beats(&dir, 12).expect("written");
    assert_eq!(load_beats(&dir), Some(12));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_the_layers_leaves_the_tempo_alone() {
    // It's written on its own, once known - a layer save after it, for
    // an overdub, mustn't take it away.
    let dir = temp_dir("mirror-beats-layers");
    let (mut input, mut mirror) = new_mirror();
    record(&mut input, &mut mirror, LoopState::Recording, &s(&[1, 2]), 2, 0);
    save_beats(&dir, 4).expect("written");

    mirror.save(&dir).expect("saved");

    assert_eq!(load_beats(&dir), Some(4));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleting_the_loop_takes_the_tempo_with_it() {
    let dir = temp_dir("mirror-beats-delete");
    save_beats(&dir, 4).expect("written");
    delete(&dir);
    assert_eq!(load_beats(&dir), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_tempo_file_that_does_not_read_is_no_tempo() {
    let dir = temp_dir("mirror-beats-bad");
    std::fs::create_dir_all(&dir).unwrap();
    for text in ["beats = \"twelve\"\n", "beats = 0\n", "not toml at all"] {
        std::fs::write(dir.join("tempo.toml"), text).unwrap();
        assert_eq!(load_beats(&dir), None, "{text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recording_straight_into_a_dub_makes_two_takes() {
    // Capturing never stops between them, but they are two layers.
    let (mut input, mut mirror) = new_mirror();
    mirror.tick(LoopState::Recording, 0, 0);
    input.push_slice(&s(&[1, 2]));
    mirror.tick(LoopState::Recording, 0, 0);
    mirror.tick(LoopState::Overdubbing, 0, 0);
    input.push_slice(&s(&[3, 4]));
    mirror.tick(LoopState::Overdubbing, 2, 0);
    mirror.tick(LoopState::Looping, 2, 0);
    mirror.tick(LoopState::Looping, 2, 0);

    assert_eq!(mirror.layers(), vec![s(&[1, 2]), s(&[3, 4])]);
}
