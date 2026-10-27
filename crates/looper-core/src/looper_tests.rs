use super::*;

use looper_hal::mock::{self, MockDriver, mock, mock_with};
use looper_hal::{BackendId, HalError};

use crate::test_support::temp_dir;

/// Small enough to follow by hand, and a rate the mock device offers.
const RATE: u32 = 44_100;
const PREROLL_MS: u32 = 1_000;

fn settings() -> AppConfig {
    AppConfig {
        backend: BackendId::MOCK.as_str().to_string(),
        device_name: mock::DEVICE.to_string(),
        sample_rate: RATE,
        preroll_ms: PREROLL_MS,
        // Short enough that a test can hold past it deliberately.
        long_press_ms: 500,
        ..AppConfig::default()
    }
}

/// A count-in to start the first take, rather than the delay.
fn counted_in() -> AppConfig {
    AppConfig {
        record_start: config::RecordStart::CountIn,
        ..settings()
    }
}

/// A looper on a device that isn't there, so nothing this does can reach
/// an interface or the loop of whoever is running the tests.
fn start(name: &str, settings: &AppConfig) -> (LooperState, MockDriver, PathBuf) {
    let (backend, driver) = mock();
    let backends = Backends::new(vec![Box::new(backend)]);
    let dir = temp_dir(name);
    let looper =
        LooperState::start_in(&backends, settings, dir.clone()).expect("the mock device opens");
    (looper, driver, dir)
}

/// One press: down on a frame, up on the next. The press itself is the
/// first frame; the second lets go, so the next press counts as new.
fn press(looper: &mut LooperState, down: Instant, up: Instant) {
    looper.tick(true, down);
    looper.tick(false, up);
}

/// S held past the threshold, then let go: a stop, then a clear.
fn hold_stop(looper: &mut LooperState, at: Instant) {
    looper.set_stop_key_held(true);
    looper.tick(false, at);
    looper.tick(false, at + Duration::from_millis(600));
    looper.set_stop_key_held(false);
    looper.tick(false, at + Duration::from_millis(700));
}

#[test]
fn starting_opens_a_stream_and_dropping_it_stops() {
    let (looper, driver, dir) = start("looper-open", &settings());
    assert!(driver.is_running(), "the device is open");
    assert_eq!(looper.state(), LoopState::Idle);

    drop(looper);

    assert!(!driver.is_running(), "and closes with the screen");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_device_that_is_not_there_says_so_instead_of_panicking() {
    let (backend, _driver) = mock();
    let backends = Backends::new(vec![Box::new(backend)]);
    let missing = AppConfig {
        device_name: "Some Interface That Went Away".to_string(),
        ..settings()
    };

    // `LooperState` holds a live stream, so it is deliberately not
    // `Debug` - which rules out `expect_err` here.
    match LooperState::start_in(&backends, &missing, temp_dir("looper-missing")) {
        Err(err) => assert!(err.contains("not found"), "got: {err}"),
        Ok(_) => panic!("there is no such device"),
    }
}

#[test]
fn an_output_with_no_channels_is_refused_rather_than_run() {
    let (backend, _driver) = mock_with(1, 0);
    let backends = Backends::new(vec![Box::new(backend)]);

    match LooperState::start_in(&backends, &settings(), temp_dir("looper-no-output")) {
        Err(err) => assert!(err.contains("no channels"), "got: {err}"),
        Ok(_) => panic!("a callback with nowhere to write would panic"),
    }
}

#[test]
fn the_rate_it_reports_is_the_one_the_device_granted() {
    // Not the one that was asked for. Everything measured in samples -
    // the loop's length in seconds included - is wrong otherwise.
    let (backend, driver) = mock();
    let backends = Backends::new(vec![Box::new(backend.always_grants(48_000))]);
    let dir = temp_dir("looper-granted");
    let asked = AppConfig {
        sample_rate: 96_000,
        ..settings()
    };

    let mut looper = LooperState::start_in(&backends, &asked, dir.clone()).expect("opens");
    let start = Instant::now();
    press(&mut looper, start, start);
    assert_eq!(looper.state(), LoopState::Arming);

    // One second of a 48 kHz device, recorded after the pre-roll.
    looper.tick(false, start + Duration::from_millis(PREROLL_MS as u64));
    assert_eq!(looper.state(), LoopState::Recording);
    driver.cycle(&vec![0.0; 48_000]);
    let at = start + Duration::from_millis(PREROLL_MS as u64 + 1);
    press(&mut looper, at, at);

    looper.tick(false, at + Duration::from_millis(1));
    let (seconds, _) = looper.loop_duration_and_progress();
    assert!(
        (seconds - 1.0).abs() < 0.01,
        "a second at the granted rate, got {seconds}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// --- The pre-roll race -------------------------------------------------
//
// `tick` takes the input before it takes the expiry, so that a press
// meant to call the countdown off still wins when it lands on the very
// frame the countdown runs out. Until the looper could be built without
// a device, that rule was a comment.

#[test]
fn a_press_on_the_frame_the_pre_roll_expires_calls_it_off() {
    let (mut looper, _driver, dir) = start("looper-race-press", &settings());
    let start = Instant::now();

    press(&mut looper, start, start);
    assert_eq!(looper.state(), LoopState::Arming, "counting down");

    // Down on exactly the frame the countdown runs out.
    let expiry = start + Duration::from_millis(PREROLL_MS as u64);
    looper.tick(false, expiry - Duration::from_millis(1));
    looper.tick(true, expiry);

    assert_eq!(
        looper.state(),
        LoopState::Idle,
        "the press called it off; recording anyway would start a take nobody asked for"
    );

    // And it stays off - a cancelled countdown must not fire later.
    looper.tick(false, expiry + Duration::from_millis(500));
    assert_eq!(looper.state(), LoopState::Idle);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_pre_roll_left_alone_starts_recording_when_it_runs_out() {
    // The other side of the same rule: without a press, the expiry is
    // what has the say.
    let (mut looper, _driver, dir) = start("looper-race-alone", &settings());
    let start = Instant::now();

    press(&mut looper, start, start);
    let expiry = start + Duration::from_millis(PREROLL_MS as u64);

    looper.tick(false, expiry - Duration::from_millis(1));
    assert_eq!(looper.state(), LoopState::Arming, "not yet");

    looper.tick(false, expiry);
    assert_eq!(looper.state(), LoopState::Recording);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_countdown_readout_matches_the_frame_it_was_ticked_with() {
    // The readouts answer from the frame's own clock rather than asking
    // the time again, so a screen can't show a countdown that disagrees
    // with the state drawn beside it.
    let (mut looper, _driver, dir) = start("looper-readout", &settings());
    let start = Instant::now();
    press(&mut looper, start, start);

    looper.tick(false, start + Duration::from_millis(250));
    assert!((looper.preroll_remaining_secs() - 0.75).abs() < 1e-6);
    assert!((looper.preroll_progress() - 0.25).abs() < 1e-6);

    // Asked again with no tick between, it must not have moved.
    assert!((looper.preroll_progress() - 0.25).abs() < 1e-6);
    let _ = std::fs::remove_dir_all(&dir);
}

// --- The press cycle, with samples actually flowing ---------------------

#[test]
fn a_take_records_loops_and_clears() {
    let no_preroll = AppConfig {
        preroll_ms: 0,
        ..settings()
    };
    let (mut looper, driver, dir) = start("looper-cycle", &no_preroll);
    let mut at = Instant::now();
    // A second apart: closer, two presses are a double.
    let mut step = |looper: &mut LooperState| {
        at += Duration::from_secs(1);
        press(looper, at, at);
    };

    step(&mut looper);
    assert_eq!(
        looper.state(),
        LoopState::Recording,
        "no pre-roll, straight in"
    );

    driver.cycle(&vec![0.5; 2_048]);
    step(&mut looper);
    assert_eq!(looper.state(), LoopState::Looping);

    // The audio thread needs a callback to publish what it settled on.
    driver.cycle(&vec![0.0; 512]);
    at += Duration::from_millis(10);
    looper.tick(false, at);
    assert_eq!(looper.layer_count(), 1, "one layer, from one take");

    // Stop held past the threshold clears, after the stop its press did.
    hold_stop(&mut looper, at);
    assert_eq!(looper.state(), LoopState::Idle, "cleared");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_saved_loop_comes_back_stopped_rather_than_playing() {
    let settings = settings();
    let (first, driver, dir) = start("looper-restore", &settings);
    let mut looper = first;
    let at = Instant::now();

    // Record something and let it be written.
    press(&mut looper, at, at);
    looper.tick(false, at + Duration::from_millis(PREROLL_MS as u64));
    driver.cycle(&vec![0.25; 4_096]);
    let stop = at + Duration::from_millis(PREROLL_MS as u64 + 10);
    press(&mut looper, stop, stop);
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, stop + Duration::from_millis(10));
    drop(looper);

    // Opening again on the same directory finds it.
    let (backend, driver) = mock();
    let backends = Backends::new(vec![Box::new(backend)]);
    let restored = LooperState::start_in(&backends, &settings, dir.clone()).expect("opens");

    assert!(driver.is_running());
    assert_eq!(
        restored.state(),
        LoopState::Stopped,
        "there, but silent until it is asked for"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// --- When the device goes away ------------------------------------------

#[test]
fn a_stream_that_fails_says_so_on_the_screen() {
    // A release build has no console, so a stream that stops without
    // this leaves the app looking like it is working and silent.
    let (mut looper, driver, dir) = start("looper-fault", &settings());
    assert_eq!(looper.device_fault(), None, "nothing wrong yet");

    driver.fail(HalError::Runtime("the device was unplugged".to_string()));

    let reported = looper
        .device_fault()
        .expect("the screen has something to show");
    assert!(reported.contains("unplugged"), "got: {reported}");

    // And it keeps saying so: the stream is gone, and only reopening a
    // device puts that right.
    looper.tick(false, Instant::now());
    assert!(looper.device_fault().is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_fault_on_one_looper_does_not_follow_the_next() {
    // Choosing a device again in Settings builds a new looper, which
    // must start clean rather than inheriting the last one's failure.
    let settings = settings();
    let (looper, driver, dir) = start("looper-fault-reset", &settings);
    driver.fail(HalError::Runtime("gone".to_string()));
    assert!(looper.device_fault().is_some());
    drop(looper);

    let (fresh, _driver, _dir) = start("looper-fault-reset-2", &settings);
    assert_eq!(fresh.device_fault(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn underruns_are_counted_for_the_screen_rather_than_only_logged() {
    // The console they used to go to doesn't exist in a release build,
    // so a player whose latency is set too low heard the gaps and had
    // nothing telling them which setting to reach for.
    let (mut looper, driver, dir) = start("looper-underruns", &settings());
    assert_eq!(looper.underruns(), 0);

    // Capture with nothing draining it: the passthrough ring fills and
    // then spills, which is the output side having fallen behind.
    driver.capture(&vec![0.0; 200_000]);
    looper.tick(false, Instant::now());

    assert!(looper.underruns() > 0, "the screen has something to report");

    // They accumulate: the count is since this looper started, not
    // since the last frame, so a burst isn't shown and instantly lost.
    let after_first = looper.underruns();
    looper.tick(false, Instant::now());
    assert_eq!(looper.underruns(), after_first, "and don't reset when read");
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Mini / extended ----------------------------------------------------

#[test]
fn switching_mode_flips_it_and_saves_it() {
    let (mut looper, _driver, dir) = start("looper-mode", &settings());
    assert_eq!(looper.mode(), Mode::Mini, "the default");

    looper.toggle_mode();

    assert_eq!(looper.mode(), Mode::Extended);
    let saved = AppConfig::load_from(&config::config_path_in(&dir)).expect("written");
    assert_eq!(
        saved.mode,
        Mode::Extended,
        "the app reopens the way it was left"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_mode_is_locked_once_anything_is_under_way() {
    // Arming counts: the pre-roll is the start of a recording.
    let (mut looper, _driver, dir) = start("looper-mode-locked", &settings());
    let at = Instant::now();
    press(&mut looper, at, at);
    assert_eq!(looper.state(), LoopState::Arming);

    assert!(!looper.can_switch_mode());
    looper.toggle_mode();

    assert_eq!(looper.mode(), Mode::Mini, "unchanged");
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Layer mute -----------------------------------------------------------

/// A looper playing two layers: a first take and one overdub.
fn two_layers(name: &str) -> (LooperState, MockDriver, PathBuf, Instant) {
    let no_preroll = AppConfig {
        preroll_ms: 0,
        ..settings()
    };
    let (mut looper, driver, dir) = start(name, &no_preroll);
    let mut at = Instant::now();
    single(&mut looper, &mut at);
    driver.cycle(&vec![0.5; 2_048]);
    single(&mut looper, &mut at);
    driver.cycle(&vec![0.0; 512]);

    // Ticked between the steps, as frames would: the UI thread's copy of
    // the loop only sees a take it's ticked through.
    single(&mut looper, &mut at);
    driver.cycle(&vec![0.25; 512]);
    single(&mut looper, &mut at);
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);
    assert_eq!(looper.layer_count(), 2, "set up");
    (looper, driver, dir, at)
}

#[test]
fn a_finished_layer_mutes_and_unmutes() {
    let (mut looper, _driver, dir, _) = two_layers("looper-mute");

    looper.toggle_layer_mute(1);
    assert!(looper.is_layer_muted(1));
    assert!(!looper.is_layer_muted(0), "only the one asked for");

    looper.toggle_layer_mute(1);
    assert!(!looper.is_layer_muted(1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_layer_that_does_not_exist_cannot_be_muted() {
    let (mut looper, _driver, dir, _) = two_layers("looper-mute-missing");
    looper.toggle_layer_mute(2);
    assert!(!looper.is_layer_muted(2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn removing_a_muted_layer_frees_its_slot_unmuted() {
    // Otherwise the next overdub, recorded into the same slot, would be
    // silent from the moment it was finished.
    let (mut looper, _driver, dir, _) = two_layers("looper-mute-remove");
    looper.toggle_layer_mute(0);
    looper.toggle_layer_mute(1);

    looper.remove_last_layer();

    assert!(!looper.is_layer_muted(1), "the freed slot");
    assert!(
        looper.is_layer_muted(0),
        "the layer still there keeps its mute"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clearing_forgets_every_mute() {
    let (mut looper, _driver, dir, at) = two_layers("looper-mute-clear");
    looper.toggle_layer_mute(0);

    hold_stop(&mut looper, at);
    assert_eq!(looper.state(), LoopState::Idle, "cleared");

    assert!(!looper.is_layer_muted(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn muting_every_layer_silences_the_output() {
    // The mask has to cross to the audio thread, not just flip here.
    let (mut looper, driver, dir, _) = two_layers("looper-mute-output");
    let silence = vec![0.0; 512];
    assert!(driver.cycle(&silence).iter().any(|&s| s != 0.0), "playing");

    looper.toggle_layer_mute(0);
    looper.toggle_layer_mute(1);
    assert!(driver.cycle(&silence).iter().all(|&s| s == 0.0), "silent");

    looper.toggle_layer_mute(1);
    assert!(driver.cycle(&silence).iter().any(|&s| s != 0.0), "one back");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_stopped_loop_still_lets_the_mode_switch() {
    // A saved loop always comes back stopped; clearing it just to switch
    // would cost the loop.
    let (mut looper, _driver, dir, at) = two_layers("looper-mode-stopped");
    assert!(!looper.can_switch_mode(), "locked while it plays");

    press(&mut looper, at, at);
    assert_eq!(looper.state(), LoopState::Stopped);
    looper.toggle_mode();

    assert_eq!(looper.mode(), Mode::Extended);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn going_back_to_mini_unmutes_every_layer() {
    // Mini shows no layer row, so a mute kept there would be silence
    // with nothing on screen to explain it.
    let (mut looper, _driver, dir, at) = two_layers("looper-mode-unmute");
    press(&mut looper, at, at);
    looper.toggle_mode();
    looper.toggle_layer_mute(0);

    looper.toggle_mode();

    assert_eq!(looper.mode(), Mode::Mini);
    assert!(!looper.is_layer_muted(0));
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Redo -----------------------------------------------------------------

#[test]
fn redo_brings_a_removed_layer_back_and_saves_it() {
    let (mut looper, driver, dir, at) = two_layers("looper-redo");
    looper.remove_last_layer();
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);
    assert_eq!(looper.layer_count(), 1);
    assert!(looper.can_redo_layer());

    looper.redo_layer();
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);

    assert_eq!(looper.layer_count(), 2, "back on the audio thread");
    assert!(!looper.can_redo_layer());
    looper.writer.flush();
    assert!(
        config::loop_dir_in(&dir).join("layer-2.wav").exists(),
        "and on disk"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_overdub_after_a_remove_ends_the_chance_to_redo() {
    let (mut looper, driver, dir, mut at) = two_layers("looper-redo-overdub");
    looper.remove_last_layer();
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);

    single(&mut looper, &mut at);
    driver.cycle(&vec![0.25; 512]);
    single(&mut looper, &mut at);

    assert!(!looper.can_redo_layer());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn undo_never_takes_the_first_take() {
    // Emptying the loop is a clear's job.
    let (mut looper, driver, dir) = start("looper-undo-first", &rec_play(4));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    assert!(!looper.can_remove_layer());

    looper.remove_last_layer();
    feed(&mut looper, &driver, 512, at);

    assert_eq!(looper.state(), LoopState::Looping);
    assert_eq!(looper.layer_count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_long_press_clear_is_final() {
    let (mut looper, _driver, dir, at) = two_layers("looper-redo-clear");
    looper.remove_last_layer();

    hold_stop(&mut looper, at);
    assert_eq!(looper.state(), LoopState::Idle, "cleared");

    assert!(!looper.can_redo_layer());
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Metronome ------------------------------------------------------------

/// Whether anything sounds in `frames` of output, with nothing played in.
fn sounds(driver: &MockDriver, frames: usize) -> bool {
    driver.cycle(&vec![0.0; frames]).iter().any(|&s| s != 0.0)
}

#[test]
fn the_metronome_is_off_until_asked_for() {
    let (mut looper, driver, dir) = start("looper-metronome", &settings());
    assert!(!looper.is_metronome_on());
    assert!(!sounds(&driver, 2_048), "silence");

    looper.toggle_metronome();

    assert!(sounds(&driver, 2_048), "a click, with no loop at all");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_count_in_clicks_from_beat_one() {
    let (mut looper, driver, dir) = start("looper-metronome-arming", &counted_in());
    looper.toggle_metronome();
    sounds(&driver, 30_000);

    let at = Instant::now();
    press(&mut looper, at, at);
    assert_eq!(looper.state(), LoopState::Arming);

    // Restarted at beat one by the pre-roll, not half a beat on.
    let out = driver.cycle(&vec![0.0; 512]);
    assert_eq!(out[0], 0.0, "the click's first sample");
    assert!(out[1..64].iter().any(|&s| s != 0.0), "a click right away");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn turning_the_metronome_off_silences_it() {
    let (mut looper, driver, dir) = start("looper-metronome-off", &settings());
    looper.toggle_metronome();
    looper.toggle_metronome();
    assert!(!sounds(&driver, 30_000));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_tempo_changes_live_and_is_saved() {
    let (mut looper, driver, dir) = start("looper-tempo", &settings());
    looper.toggle_metronome();

    looper.change_metronome_bpm(180);

    assert_eq!(looper.tempo(), 300.0);
    let saved = AppConfig::load_from(&config::config_path_in(&dir)).expect("written");
    assert_eq!(
        saved.metronome_bpm, 300,
        "Settings and the next start agree"
    );

    // At 300 BPM a beat is 8,820 frames, so three clicks fit in 20,000
    // where 120 BPM would fit one. Each click opens on a zero sample after
    // silence, so count those openings.
    let out = driver.cycle(&vec![0.0; 20_000]);
    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let clicks = left
        .windows(2)
        .filter(|pair| pair[0] == 0.0 && pair[1] != 0.0)
        .count();
    assert!(clicks >= 3, "got {clicks}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tapping_sets_the_tempo_and_saves_it() {
    let (mut looper, _driver, dir) = start("looper-tap", &settings());
    let t0 = Instant::now();
    looper.tap_tempo(t0);
    assert_eq!(
        looper.tempo(),
        f64::from(config::DEFAULT_METRONOME_BPM),
        "one tap: nothing yet"
    );

    looper.tap_tempo(t0 + Duration::from_millis(600));
    looper.tap_tempo(t0 + Duration::from_millis(1_190));

    // Gaps of 600 and 590 ms: 595 on average, 100.8 BPM, whole.
    assert_eq!(looper.tempo(), 101.0);
    let saved = AppConfig::load_from(&config::config_path_in(&dir)).expect("written");
    assert_eq!(saved.metronome_bpm, 101);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tapping_is_ignored_while_a_loop_sets_the_tempo() {
    let (mut looper, _driver, dir, at) = free_take("looper-tap-loop", 88_200);
    let tempo = looper.tempo();

    looper.tap_tempo(at);
    looper.tap_tempo(at + Duration::from_millis(300));

    assert_eq!(looper.tempo(), tempo, "the loop's, still");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn levels_are_heard_at_once_and_saved_when_asked() {
    let (mut looper, driver, dir, _) = two_layers("looper-levels");
    let silence = vec![0.0; 512];
    assert!(driver.cycle(&silence).iter().any(|&s| s != 0.0), "playing");

    looper.set_loop_volume_pct(0);
    looper.set_click_volume_pct(40);
    assert!(
        driver.cycle(&silence).iter().all(|&s| s == 0.0),
        "the loop silenced"
    );
    let before = AppConfig::load_from(&config::config_path_in(&dir));
    assert_ne!(before.map(|c| c.volume_pct), Some(0), "not saved mid-drag");

    looper.save_levels();

    let saved = AppConfig::load_from(&config::config_path_in(&dir)).expect("written");
    assert_eq!((saved.volume_pct, saved.click_volume_pct), (0, 40));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_tempo_stays_inside_its_range() {
    let (mut looper, _driver, dir) = start("looper-tempo-range", &settings());
    looper.change_metronome_bpm(-1_000);
    assert_eq!(looper.tempo(), f64::from(*METRONOME_BPM_RANGE.start()));
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Loops in whole bars --------------------------------------------------

/// 120 BPM at 44.1 kHz.
const BAR: usize = 88_200;

/// Recording with the metronome on: through the bar's count-in, then
/// `frames` into the take, then a press to stop.
fn record_against_the_click(
    name: &str,
    frames: usize,
) -> (LooperState, MockDriver, PathBuf, Instant) {
    let (mut looper, driver, dir) = start(name, &counted_in());
    looper.toggle_metronome();
    let mut at = Instant::now();
    press(&mut looper, at, at);
    feed(&mut looper, &driver, BAR, at);
    feed(&mut looper, &driver, frames, at);
    assert_eq!(looper.state(), LoopState::Recording, "counted in");
    single(&mut looper, &mut at);
    (looper, driver, dir, at)
}

/// `frames` of input, a frame tick after each buffer as the app would:
/// the UI thread's copy drains a ring that holds only two seconds.
fn feed(looper: &mut LooperState, driver: &MockDriver, frames: usize, at: Instant) {
    for _ in 0..frames / 4_096 {
        driver.cycle(&vec![0.25; 4_096]);
        looper.tick(false, at);
    }
    driver.cycle(&vec![0.25; frames % 4_096]);
    looper.tick(false, at);
}

#[test]
fn stopping_late_cuts_the_loop_back_to_the_bar() {
    let (mut looper, driver, dir, at) =
        record_against_the_click("looper-bars-late", 3 * BAR + BAR / 3);
    assert_eq!(looper.state(), LoopState::Looping, "stops straight away");
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);

    let (secs, _) = looper.loop_duration_and_progress();
    assert_eq!(secs, 6.0, "three bars of 2 s");
    looper.writer.flush();
    let (saved, _) =
        crate::wav::read(&config::loop_dir_in(&dir).join("layer-1.wav")).expect("saved");
    assert_eq!(saved.len(), 3 * BAR, "and saved that way, not wrapped");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stopping_early_records_on_to_the_bar_line() {
    let (mut looper, driver, dir, at) =
        record_against_the_click("looper-bars-early", 3 * BAR + 2 * BAR / 3);
    assert_eq!(looper.state(), LoopState::Recording, "not yet");
    assert!(looper.is_finishing_bar());

    feed(&mut looper, &driver, BAR / 3 + 1_000, at);
    assert_eq!(looper.state(), LoopState::Looping, "past the line");
    assert!(!looper.is_finishing_bar());

    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);
    let (secs, _) = looper.loop_duration_and_progress();
    assert_eq!(secs, 8.0, "four bars");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_press_stops_without_waiting() {
    let (mut looper, _driver, dir, mut at) =
        record_against_the_click("looper-bars-now", 3 * BAR + 2 * BAR / 3);
    single(&mut looper, &mut at);
    assert_eq!(looper.state(), LoopState::Looping);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn with_the_metronome_off_a_loop_is_as_long_as_it_was_played() {
    let no_preroll = AppConfig {
        preroll_ms: 0,
        ..settings()
    };
    let (mut looper, driver, dir) = start("looper-bars-off", &no_preroll);
    let mut at = Instant::now();
    press(&mut looper, at, at);
    feed(&mut looper, &driver, BAR + 1_000, at);
    single(&mut looper, &mut at);
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);

    let (secs, _) = looper.loop_duration_and_progress();
    assert!(
        (secs - (BAR + 1_000) as f32 / 44_100.0).abs() < 0.02,
        "got {secs}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Count-in ---------------------------------------------------------------

#[test]
fn a_count_in_is_one_bar_not_the_record_delay() {
    // The configured delay is a second; the bar is two. No metronome:
    // a count-in clicks without it.
    let (mut looper, driver, dir) = start("looper-count-in", &counted_in());
    let at = Instant::now();
    press(&mut looper, at, at);
    assert_eq!(looper.state(), LoopState::Arming);

    // Past the record delay, well short of the bar: still counting,
    // however much time the frames say has gone by.
    feed(&mut looper, &driver, BAR / 2, at);
    looper.tick(false, at + Duration::from_millis(PREROLL_MS as u64 + 100));
    assert_eq!(looper.state(), LoopState::Arming);
    assert!(
        (looper.preroll_remaining_secs() - 0.9).abs() < 0.01,
        "a bar of 2 s"
    );

    feed(&mut looper, &driver, BAR / 2 + 1_000, at);
    assert_eq!(looper.state(), LoopState::Recording, "on the bar line");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn in_three_four_the_count_in_is_three_beats() {
    // A beat at 120 is 22,050 frames; three of them, not four.
    let three_four = AppConfig {
        time_signature: config::TimeSignature::ThreeFour,
        ..counted_in()
    };
    let (mut looper, driver, dir) = start("looper-count-in-3-4", &three_four);
    let at = Instant::now();
    press(&mut looper, at, at);

    feed(&mut looper, &driver, 3 * 22_050 - 1_000, at);
    assert_eq!(looper.state(), LoopState::Arming);
    feed(&mut looper, &driver, 2_000, at);

    assert_eq!(
        looper.state(),
        LoopState::Recording,
        "on the third beat's end"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn turning_the_metronome_off_leaves_a_count_in_counting() {
    let (mut looper, driver, dir) = start("looper-count-in-off", &counted_in());
    looper.toggle_metronome();
    let at = Instant::now();
    press(&mut looper, at, at);

    looper.toggle_metronome();
    feed(&mut looper, &driver, BAR + 1_000, at);

    assert_eq!(
        looper.state(),
        LoopState::Recording,
        "on the bar line all the same"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Record start -----------------------------------------------------------

fn starting(record_start: config::RecordStart) -> AppConfig {
    AppConfig {
        record_start,
        ..settings()
    }
}

#[test]
fn instant_records_as_space_goes_down() {
    // A record delay is set, and ignored.
    let (mut looper, _driver, dir) = start(
        "looper-start-instant",
        &starting(config::RecordStart::Instant),
    );
    let at = Instant::now();
    press(&mut looper, at, at);
    assert_eq!(looper.state(), LoopState::Recording);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn with_the_metronome_on_a_delay_is_still_a_delay() {
    // No longer turned into a count-in: that's a setting of its own.
    let (mut looper, _driver, dir) = start("looper-start-delay-click", &settings());
    looper.toggle_metronome();
    let at = Instant::now();
    press(&mut looper, at, at);
    assert_eq!(looper.state(), LoopState::Arming);

    looper.tick(false, at + Duration::from_millis(PREROLL_MS as u64));

    assert_eq!(
        looper.state(),
        LoopState::Recording,
        "when the delay runs out"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn detect_waits_for_the_first_note() {
    let (mut looper, driver, dir) = start(
        "looper-start-detect",
        &starting(config::RecordStart::Detect),
    );
    let at = Instant::now();
    press(&mut looper, at, at);
    assert!(looper.is_waiting_to_hear());

    // However long it's quiet for.
    for _ in 0..20 {
        driver.cycle(&vec![0.0; 4_096]);
        looper.tick(false, at + Duration::from_secs(10));
    }
    assert_eq!(looper.state(), LoopState::Arming);

    feed(&mut looper, &driver, 4_096, at);

    assert_eq!(looper.state(), LoopState::Recording);
    assert!(!looper.is_waiting_to_hear());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_press_calls_off_waiting_for_the_first_note() {
    let (mut looper, driver, dir) = start(
        "looper-start-detect-off",
        &starting(config::RecordStart::Detect),
    );
    let mut at = Instant::now();
    press(&mut looper, at, at);

    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 4_096, at);

    assert_eq!(
        looper.state(),
        LoopState::Idle,
        "and playing doesn't start it"
    );
    assert_eq!(looper.loop_duration_and_progress().0, 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Tempo from the loop ----------------------------------------------------

/// A first take of `frames` with the metronome off, stopped and settled.
/// The loop plays silent, so a click is all the output carries.
fn free_take(name: &str, frames: usize) -> (LooperState, MockDriver, PathBuf, Instant) {
    let no_preroll = AppConfig {
        preroll_ms: 0,
        volume_pct: 0,
        ..settings()
    };
    let (mut looper, driver, dir) = start(name, &no_preroll);
    let mut at = Instant::now();
    press(&mut looper, at, at);
    feed(&mut looper, &driver, frames, at);
    single(&mut looper, &mut at);
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);
    (looper, driver, dir, at)
}

#[test]
fn a_loop_played_without_the_click_gets_a_tempo_from_its_length() {
    // 2.6 s is four beats at 92.3.
    let (looper, _driver, dir, _) = free_take("looper-tempo-guess", 114_660);
    assert!(looper.tempo_is_the_loops());
    assert!(
        (looper.tempo() - 92.3).abs() < 0.1,
        "got {}",
        looper.tempo()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_loop_played_to_the_click_keeps_the_clicks_tempo() {
    // Three bars at 120: the guess would have made it 160 or 80.
    let (mut looper, driver, dir, at) = record_against_the_click("looper-tempo-click", 3 * BAR);
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);
    assert!(looper.tempo_is_the_loops());
    assert!(
        (looper.tempo() - 120.0).abs() < 0.01,
        "got {}",
        looper.tempo()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wrong_guess_is_halved_or_doubled_but_not_nudged() {
    let (mut looper, _driver, dir, _) = free_take("looper-tempo-fix", 88_200);
    assert!(
        (looper.tempo() - 120.0).abs() < 0.01,
        "two seconds, four beats"
    );

    looper.change_metronome_bpm(5);
    assert!((looper.tempo() - 120.0).abs() < 0.01, "the loop fixes it");

    looper.halve_tempo();
    assert!((looper.tempo() - 60.0).abs() < 0.01);
    looper.double_tempo();
    looper.double_tempo();
    assert!((looper.tempo() - 240.0).abs() < 0.01);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_loops_tempo_reaches_the_click() {
    // Halved to 60 at 44.1 kHz, a beat is 44,100 frames: two or three
    // clicks start in 90,000 frames, where 120 would start four or five.
    let (mut looper, driver, dir, at) = free_take("looper-tempo-click-rate", 88_200);
    looper.halve_tempo();
    looper.toggle_metronome();
    looper.tick(false, at);

    let out = driver.cycle(&vec![0.0; 90_000]);
    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let clicks = left
        .windows(2)
        .filter(|pair| pair[0] == 0.0 && pair[1] != 0.0)
        .count();
    assert!((2..=3).contains(&clicks), "got {clicks}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clearing_the_loop_hands_the_tempo_back() {
    let (mut looper, _driver, dir, at) = free_take("looper-tempo-clear", 114_660);
    hold_stop(&mut looper, at);
    assert_eq!(looper.state(), LoopState::Idle);

    assert!(!looper.tempo_is_the_loops());
    assert_eq!(looper.tempo(), f64::from(config::DEFAULT_METRONOME_BPM));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_saved_loop_comes_back_with_a_tempo() {
    let (looper, _driver, dir, _) = free_take("looper-tempo-restore", 88_200);
    drop(looper);

    let (backend, driver) = mock();
    let backends = Backends::new(vec![Box::new(backend)]);
    let mut restored = LooperState::start_in(&backends, &settings(), dir.clone()).expect("opens");
    driver.cycle(&vec![0.0; 512]);
    restored.tick(false, Instant::now());

    assert!(restored.tempo_is_the_loops());
    assert!(
        (restored.tempo() - 120.0).abs() < 0.01,
        "got {}",
        restored.tempo()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_halved_tempo_survives_a_restart() {
    // Guessed at 120, halved to 60: a restart must not guess 120 again.
    let (mut looper, _driver, dir, _) = free_take("looper-tempo-saved", 88_200);
    looper.halve_tempo();
    drop(looper);

    let restored = reopen(&dir);

    assert!(
        (restored.tempo() - 60.0).abs() < 0.01,
        "got {}",
        restored.tempo()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_loop_played_to_the_click_comes_back_at_the_clicks_tempo() {
    // Three bars at 120 is six seconds - which a guess would call 80.
    let (mut looper, driver, dir, at) =
        record_against_the_click("looper-tempo-click-saved", 3 * BAR);
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);
    drop(looper);

    let restored = reopen(&dir);

    assert!(
        (restored.tempo() - 120.0).abs() < 0.01,
        "got {}",
        restored.tempo()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_saved_tempo_the_click_cannot_play_is_guessed_instead() {
    let (looper, _driver, dir, _) = free_take("looper-tempo-saved-bad", 88_200);
    drop(looper);
    // 1,000 beats in two seconds is 30,000 BPM.
    loop_mirror::save_beats(&config::loop_dir_in(&dir), 1_000).unwrap();

    let restored = reopen(&dir);

    assert!(
        (restored.tempo() - 120.0).abs() < 0.01,
        "got {}",
        restored.tempo()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clearing_deletes_the_saved_tempo() {
    let (mut looper, _driver, dir, at) = free_take("looper-tempo-saved-clear", 88_200);
    looper.writer.flush();
    assert!(
        loop_mirror::load_beats(&config::loop_dir_in(&dir)).is_some(),
        "saved"
    );

    hold_stop(&mut looper, at);
    looper.writer.flush();

    assert_eq!(loop_mirror::load_beats(&config::loop_dir_in(&dir)), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Opens a looper again on `dir`, as the next launch would, and lets it
/// learn the restored loop's length.
fn reopen(dir: &std::path::Path) -> LooperState {
    let (backend, driver) = mock();
    let backends = Backends::new(vec![Box::new(backend)]);
    let mut looper =
        LooperState::start_in(&backends, &settings(), dir.to_path_buf()).expect("opens");
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, Instant::now());
    looper
}

// --- Record modes -------------------------------------------------------------

fn rec_play(max_layers: u32) -> AppConfig {
    AppConfig {
        preroll_ms: 0,
        max_layers,
        ..settings()
    }
}

fn rec_dub(max_layers: u32) -> AppConfig {
    AppConfig {
        record_mode: config::RecordMode::RecDub,
        ..rec_play(max_layers)
    }
}

fn rec_dub_per_pass(max_layers: u32) -> AppConfig {
    AppConfig {
        record_mode: config::RecordMode::RecDubPerPass,
        ..rec_play(max_layers)
    }
}

/// Two presses close enough together to make a double, with a block of
/// audio between them: whatever the first half opened is recording by
/// the time the second arrives.
fn double_press(looper: &mut LooperState, driver: &MockDriver, at: &mut Instant) {
    *at += Duration::from_secs(1);
    looper.tick(true, *at);
    driver.cycle(&vec![0.5; 2_048]);
    looper.tick(false, *at + Duration::from_millis(50));
    looper.tick(true, *at + Duration::from_millis(150));
    looper.tick(false, *at + Duration::from_millis(200));
    feed(looper, driver, 512, *at);
}

/// Single presses, far enough apart never to make a double.
fn single(looper: &mut LooperState, at: &mut Instant) {
    *at += Duration::from_secs(1);
    press(looper, *at, *at);
}

/// A first take, ended with a press - so playing in REC→PLAY, dubbing in
/// REC→DUB.
fn take(looper: &mut LooperState, driver: &MockDriver, at: &mut Instant) {
    single(looper, at);
    feed(looper, driver, 8_000, *at);
    single(looper, at);
    feed(looper, driver, 512, *at);
}

#[test]
fn rec_play_walks_record_play_dub_play() {
    let (mut looper, driver, dir) = start("looper-rec-play", &rec_play(4));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    assert_eq!(looper.state(), LoopState::Looping, "the take plays");

    single(&mut looper, &mut at);
    assert_eq!(
        looper.state(),
        LoopState::Overdubbing,
        "a press while playing dubs"
    );
    feed(&mut looper, &driver, 4_000, at);
    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 512, at);

    assert_eq!(looper.state(), LoopState::Looping, "and the next plays");
    assert_eq!(looper.layer_count(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn with_every_layer_used_a_press_does_nothing() {
    let (mut looper, driver, dir) = start("looper-rec-play-full", &rec_play(2));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 4_000, at);
    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 512, at);
    assert_eq!(looper.layer_count(), 2, "both used");

    single(&mut looper, &mut at);

    assert_eq!(looper.state(), LoopState::Looping, "a press never stops");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_press_plays_a_stopped_loop() {
    let (mut looper, driver, dir) = start("looper-press-plays", &rec_play(4));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    looper.stop();

    single(&mut looper, &mut at);

    assert_eq!(looper.state(), LoopState::Looping);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rec_dub_dubs_layer_after_layer_without_playing_between() {
    let (mut looper, driver, dir) = start("looper-rec-dub", &rec_dub(4));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    assert_eq!(
        looper.state(),
        LoopState::Overdubbing,
        "straight into the second part"
    );
    feed(&mut looper, &driver, 4_000, at);

    single(&mut looper, &mut at);
    assert_eq!(
        looper.state(),
        LoopState::Overdubbing,
        "the next layer, not play"
    );
    feed(&mut looper, &driver, 4_000, at);
    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 4_000, at);
    looper.stop();
    feed(&mut looper, &driver, 512, at);

    assert_eq!(
        looper.state(),
        LoopState::Looping,
        "S steps down to playing"
    );
    assert_eq!(looper.layer_count(), 4, "every press a layer of its own");
    looper.writer.flush();
    assert!(
        config::loop_dir_in(&dir).join("layer-4.wav").exists(),
        "and saved split where the audio thread split it"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rec_dub_with_no_layer_left_after_this_one_plays() {
    let (mut looper, driver, dir) = start("looper-rec-dub-last", &rec_dub(2));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    feed(&mut looper, &driver, 4_000, at);
    assert!(!looper.dubs_on());

    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 512, at);

    assert_eq!(looper.state(), LoopState::Looping);
    assert_eq!(looper.layer_count(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_layer_per_pass_stacks_layers_with_no_press() {
    // The take is 8,000 frames; two and a half passes of dub after it.
    let (mut looper, driver, dir) = start("looper-per-pass", &rec_dub_per_pass(8));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    assert_eq!(
        looper.state(),
        LoopState::Overdubbing,
        "straight into the second part"
    );

    feed(&mut looper, &driver, 20_000, at);
    assert_eq!(looper.layer_count(), 3, "two passes done, a third going");
    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 512, at);

    assert_eq!(looper.state(), LoopState::Looping, "a press plays");
    assert_eq!(looper.layer_count(), 4);
    looper.writer.flush();
    assert!(
        config::loop_dir_in(&dir).join("layer-4.wav").exists(),
        "saved split where the audio thread split it"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_layer_per_pass_stops_in_one_press() {
    let (mut looper, driver, dir) = start("looper-per-pass-stop", &rec_dub_per_pass(8));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    feed(&mut looper, &driver, 4_000, at);

    looper.stop();

    assert_eq!(
        looper.state(),
        LoopState::Stopped,
        "no step down, unlike REC>DUB"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn with_one_layer_allowed_there_is_nothing_to_dub_into() {
    let (mut looper, driver, dir) = start("looper-rec-dub-one", &rec_dub(1));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    assert_eq!(looper.state(), LoopState::Looping);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_dub_after_the_take_is_saved_as_a_layer_of_its_own() {
    let (mut looper, driver, dir) = start("looper-rec-dub-saved", &rec_dub(4));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    feed(&mut looper, &driver, 4_000, at);
    looper.stop();
    feed(&mut looper, &driver, 512, at);

    looper.writer.flush();
    let loop_dir = config::loop_dir_in(&dir);
    assert!(loop_dir.join("layer-1.wav").exists());
    assert!(loop_dir.join("layer-2.wav").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Stop, and the double press that does the same -------------------------

#[test]
fn stop_ends_a_dub_and_keeps_it() {
    let (mut looper, driver, dir) = start("looper-stop-dub", &rec_play(4));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 4_000, at);

    looper.stop();
    feed(&mut looper, &driver, 512, at);

    assert_eq!(looper.state(), LoopState::Stopped);
    assert_eq!(looper.layer_count(), 2, "the dub kept");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn in_rec_dub_stop_steps_down_through_playing() {
    let (mut looper, driver, dir) = start("looper-stop-rec-dub", &rec_dub(4));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    feed(&mut looper, &driver, 4_000, at);

    looper.stop();
    assert_eq!(looper.state(), LoopState::Looping);
    looper.stop();

    assert_eq!(looper.state(), LoopState::Stopped);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stop_ends_the_take_and_keeps_it_rather_than_dubbing() {
    let (mut looper, driver, dir) = start("looper-stop-take", &rec_dub(4));
    let mut at = Instant::now();
    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 8_000, at);

    looper.stop();
    feed(&mut looper, &driver, 512, at);

    assert_eq!(looper.state(), LoopState::Stopped);
    assert_eq!(looper.layer_count(), 1);
    assert!(looper.loop_duration_and_progress().0 > 0.0, "the take kept");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stop_calls_off_a_pre_roll() {
    let (mut looper, _driver, dir) = start("looper-stop-preroll", &settings());
    let at = Instant::now();
    press(&mut looper, at, at);
    assert_eq!(looper.state(), LoopState::Arming);

    looper.stop();
    looper.tick(false, at + Duration::from_millis(PREROLL_MS as u64 * 2));

    assert_eq!(
        looper.state(),
        LoopState::Idle,
        "and it doesn't start later"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_double_press_while_playing_stops_without_a_sliver_of_a_layer() {
    // The first half opened a dub, and audio reached it before the
    // second: it's dropped on the audio thread and never saved.
    let (mut looper, driver, dir) = start("looper-double-playing", &rec_play(4));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);

    double_press(&mut looper, &driver, &mut at);

    assert_eq!(looper.state(), LoopState::Stopped);
    assert_eq!(looper.layer_count(), 1);
    looper.writer.flush();
    assert!(!config::loop_dir_in(&dir).join("layer-2.wav").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_double_press_while_dubbing_stops_and_keeps_the_layer() {
    let (mut looper, driver, dir) = start("looper-double-dubbing", &rec_play(4));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 4_000, at);

    double_press(&mut looper, &driver, &mut at);

    assert_eq!(looper.state(), LoopState::Stopped);
    assert_eq!(looper.layer_count(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn in_rec_dub_a_double_press_plays_and_keeps_the_layer_before() {
    // As S does there. The first half split the dub; the layer it
    // opened goes, the one it closed stays.
    let (mut looper, driver, dir) = start("looper-double-rec-dub", &rec_dub(4));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    feed(&mut looper, &driver, 4_000, at);

    double_press(&mut looper, &driver, &mut at);

    assert_eq!(looper.state(), LoopState::Looping);
    assert_eq!(looper.layer_count(), 2);
    looper.writer.flush();
    assert!(!config::loop_dir_in(&dir).join("layer-3.wav").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_double_press_ending_the_take_stops_rather_than_dubs() {
    let (mut looper, driver, dir) = start("looper-double-take", &rec_dub(4));
    let mut at = Instant::now();
    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 8_000, at);

    double_press(&mut looper, &driver, &mut at);

    assert_eq!(looper.state(), LoopState::Stopped);
    assert_eq!(looper.layer_count(), 1, "the take, and no dub");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_double_press_on_an_empty_looper_leaves_it_empty() {
    let (mut looper, driver, dir) = start("looper-double-empty", &rec_play(4));
    let mut at = Instant::now();

    double_press(&mut looper, &driver, &mut at);

    assert_eq!(looper.state(), LoopState::Idle);
    assert_eq!(looper.loop_duration_and_progress().0, 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_double_press_on_a_stopped_loop_leaves_it_stopped() {
    let (mut looper, driver, dir) = start("looper-double-stopped", &rec_play(4));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    looper.stop();

    double_press(&mut looper, &driver, &mut at);

    assert_eq!(looper.state(), LoopState::Stopped);
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Hold to undo, hold to clear -------------------------------------------

/// Space held past the threshold with audio running under the press, so
/// whatever it opened is recording before the hold takes it back.
fn hold_space(looper: &mut LooperState, driver: &MockDriver, at: &mut Instant) {
    *at += Duration::from_secs(1);
    looper.tick(true, *at);
    driver.cycle(&vec![0.5; 2_048]);
    looper.tick(true, *at + Duration::from_millis(600));
    looper.tick(false, *at + Duration::from_millis(700));
    *at += Duration::from_millis(700);
    // Two blocks: the undo waits for the first to drop the dub, then
    // goes over in the second.
    feed(looper, driver, 512, *at);
    feed(looper, driver, 512, *at);
}

#[test]
fn holding_space_while_playing_undoes_the_last_layer() {
    // Its press opened a dub first; that goes too, never saved.
    let (mut looper, driver, dir, mut at) = two_layers("looper-hold-undo");

    hold_space(&mut looper, &driver, &mut at);

    assert_eq!(looper.state(), LoopState::Looping, "still playing");
    assert_eq!(looper.layer_count(), 1);
    assert!(
        looper.can_redo_layer(),
        "the layer, not the dub, waits for redo"
    );
    looper.writer.flush();
    assert!(!config::loop_dir_in(&dir).join("layer-3.wav").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn each_hold_undoes_one_layer_but_never_the_first_take() {
    let (mut looper, driver, dir, mut at) = two_layers("looper-hold-twice");

    hold_space(&mut looper, &driver, &mut at);
    hold_space(&mut looper, &driver, &mut at);

    assert_eq!(looper.state(), LoopState::Looping);
    assert_eq!(looper.layer_count(), 1, "clearing is S's hold");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn holding_space_while_dubbing_undoes_the_dub_in_progress() {
    let (mut looper, driver, dir) = start("looper-hold-dubbing", &rec_play(4));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 4_000, at);

    hold_space(&mut looper, &driver, &mut at);

    assert_eq!(looper.state(), LoopState::Looping);
    assert_eq!(looper.layer_count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn in_rec_dub_holding_undoes_the_dub_in_progress_and_plays() {
    // The press split it, opening the next; both go.
    let (mut looper, driver, dir) = start("looper-hold-rec-dub", &rec_dub(4));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    feed(&mut looper, &driver, 4_000, at);

    hold_space(&mut looper, &driver, &mut at);

    assert_eq!(looper.state(), LoopState::Looping);
    assert_eq!(looper.layer_count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn holding_space_on_a_stopped_loop_undoes_and_stays_stopped() {
    let (mut looper, driver, dir, mut at) = two_layers("looper-hold-stopped");
    looper.stop();

    hold_space(&mut looper, &driver, &mut at);

    assert_eq!(
        looper.state(),
        LoopState::Stopped,
        "its press played it; taken back"
    );
    assert_eq!(looper.layer_count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn holding_space_through_the_end_of_the_first_take_empties_the_looper() {
    let (mut looper, driver, dir) = start("looper-hold-take", &rec_play(4));
    let mut at = Instant::now();
    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 8_000, at);

    hold_space(&mut looper, &driver, &mut at);

    assert_eq!(looper.state(), LoopState::Idle);
    assert_eq!(looper.loop_duration_and_progress().0, 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn holding_the_stop_button_clears_a_stopped_loop() {
    let (mut looper, _driver, dir, at) = two_layers("looper-hold-stop-button");
    looper.stop();
    assert!(
        looper.can_stop_or_clear(),
        "stopped, there's still a loop to clear"
    );

    looper.set_stop_button_held(true);
    looper.tick(false, at);
    looper.tick(false, at + Duration::from_millis(600));

    assert_eq!(looper.state(), LoopState::Idle);
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Stop mode --------------------------------------------------------------

fn stopping(stop_mode: config::StopMode) -> AppConfig {
    AppConfig {
        stop_mode,
        // The shortest fade there is: 22,050 frames.
        fade_ms: 500,
        ..rec_play(4)
    }
}

#[test]
fn a_stop_at_the_loop_end_plays_on_until_it() {
    let (mut looper, driver, dir) = start("looper-stop-end", &stopping(config::StopMode::LoopEnd));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);

    looper.stop();
    feed(&mut looper, &driver, 512, at);
    assert_eq!(looper.state(), LoopState::Looping, "still going");
    assert_eq!(looper.pending_stop(), Some(config::StopMode::LoopEnd));

    feed(&mut looper, &driver, 8_000, at);

    assert_eq!(looper.state(), LoopState::Stopped);
    assert_eq!(looper.pending_stop(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stopping_again_while_it_plays_out_stops_now() {
    let (mut looper, driver, dir) = start("looper-stop-now", &stopping(config::StopMode::LoopEnd));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);

    looper.stop();
    looper.stop();

    assert_eq!(looper.state(), LoopState::Stopped);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn space_while_it_plays_out_calls_the_stop_off_without_dubbing() {
    let (mut looper, driver, dir) = start("looper-stop-off", &stopping(config::StopMode::LoopEnd));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    looper.stop();

    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 9_000, at);

    assert_eq!(
        looper.state(),
        LoopState::Looping,
        "past the loop end, still playing"
    );
    assert_eq!(looper.layer_count(), 1, "no dub");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_dub_stopped_at_the_loop_end_is_kept_at_once() {
    let (mut looper, driver, dir) =
        start("looper-stop-end-dub", &stopping(config::StopMode::LoopEnd));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 4_000, at);

    looper.stop();
    feed(&mut looper, &driver, 512, at);
    assert_eq!(
        looper.state(),
        LoopState::Looping,
        "the dub closed, the loop playing out"
    );
    assert_eq!(looper.layer_count(), 2);

    feed(&mut looper, &driver, 8_000, at);
    assert_eq!(looper.state(), LoopState::Stopped);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_fade_out_stops_once_it_has_faded() {
    let (mut looper, driver, dir) = start("looper-stop-fade", &stopping(config::StopMode::FadeOut));
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);

    looper.stop();
    feed(&mut looper, &driver, 20_000, at);
    assert_eq!(looper.pending_stop(), Some(config::StopMode::FadeOut));
    feed(&mut looper, &driver, 4_000, at);

    assert_eq!(looper.state(), LoopState::Stopped);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ending_the_first_take_with_stop_is_always_at_once() {
    // The loop hasn't played yet: there's nothing to play out or fade.
    let (mut looper, driver, dir) =
        start("looper-stop-take-end", &stopping(config::StopMode::FadeOut));
    let mut at = Instant::now();
    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 8_000, at);

    looper.stop();

    assert_eq!(looper.state(), LoopState::Stopped);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_loop_stopped_at_its_end_plays_again_on_a_press() {
    let (mut looper, driver, dir) = start(
        "looper-stop-end-replay",
        &stopping(config::StopMode::LoopEnd),
    );
    let mut at = Instant::now();
    take(&mut looper, &driver, &mut at);
    looper.stop();
    feed(&mut looper, &driver, 8_512, at);
    assert_eq!(looper.state(), LoopState::Stopped);

    single(&mut looper, &mut at);
    feed(&mut looper, &driver, 512, at);

    assert_eq!(looper.state(), LoopState::Looping);
    assert!(
        looper.loop_duration_and_progress().1 > 0.0,
        "moving, not held at the top"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Every gesture, from every state, in every record mode ------------------

/// What the player does - and, for setting a row up, `Wait` and
/// `Record`: a quarter of a second of audio, and a first take that long
/// ended with a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gesture {
    Press,
    Double,
    Hold,
    Stop,
    StopHold,
    Wait,
    Record,
}

/// One gesture, then a little audio, so whatever it opened is recording
/// and whatever it asked of the audio thread has been carried out.
fn perform(looper: &mut LooperState, driver: &MockDriver, at: &mut Instant, gesture: Gesture) {
    match gesture {
        Gesture::Press => single(looper, at),
        Gesture::Double => double_press(looper, driver, at),
        Gesture::Hold => hold_space(looper, driver, at),
        Gesture::Stop => {
            *at += Duration::from_secs(1);
            looper.set_stop_key_held(true);
            looper.tick(false, *at);
            looper.set_stop_key_held(false);
            looper.tick(false, *at + Duration::from_millis(50));
        }
        Gesture::StopHold => {
            *at += Duration::from_secs(1);
            hold_stop(looper, *at);
            *at += Duration::from_millis(700);
        }
        // Long enough that no row's gestures add up to a whole pass,
        // which a layer per pass would turn into a layer of its own.
        Gesture::Wait => feed(looper, driver, 10_000, *at),
        Gesture::Record => {
            perform(looper, driver, at, Gesture::Press);
            perform(looper, driver, at, Gesture::Wait);
            perform(looper, driver, at, Gesture::Press);
        }
    }
    feed(looper, driver, 1_000, *at);
}

#[test]
fn every_gesture_from_every_state_in_every_mode() {
    use Gesture::*;
    use LoopState::*;
    use config::RecordMode::*;

    // How each state is reached in each mode.
    let setups: [(config::RecordMode, LoopState, &[Gesture]); 15] = [
        (RecPlay, Idle, &[]),
        (RecPlay, Recording, &[Press, Wait]),
        (RecPlay, Looping, &[Record]),
        (RecPlay, Overdubbing, &[Record, Press]),
        (RecPlay, Stopped, &[Record, Stop]),
        (RecDub, Idle, &[]),
        (RecDub, Recording, &[Press, Wait]),
        (RecDub, Overdubbing, &[Record]),
        (RecDub, Looping, &[Record, Stop]),
        (RecDub, Stopped, &[Record, Stop, Stop]),
        (RecDubPerPass, Idle, &[]),
        (RecDubPerPass, Recording, &[Press, Wait]),
        (RecDubPerPass, Overdubbing, &[Record]),
        (RecDubPerPass, Looping, &[Record, Press]),
        (RecDubPerPass, Stopped, &[Record, Stop]),
    ];
    // Mode, from, gesture -> the state it leaves and the layers kept,
    // once the audio thread has caught up. Checked against the user
    // guide's tables; a row changing is a behaviour changing.
    #[rustfmt::skip]
    let expected: &[(config::RecordMode, LoopState, Gesture, LoopState, usize)] = &[
        // REC→PLAY. One layer can't be undone - that's a clear.
        (RecPlay, Idle, Press, Recording, 0),
        (RecPlay, Idle, Double, Idle, 0),
        (RecPlay, Idle, Hold, Idle, 0),
        (RecPlay, Idle, Stop, Idle, 0),
        (RecPlay, Idle, StopHold, Idle, 0),
        (RecPlay, Recording, Press, Looping, 1),
        (RecPlay, Recording, Double, Stopped, 1),
        (RecPlay, Recording, Hold, Idle, 0),
        (RecPlay, Recording, Stop, Stopped, 1),
        (RecPlay, Recording, StopHold, Idle, 0),
        (RecPlay, Looping, Press, Overdubbing, 1),
        (RecPlay, Looping, Double, Stopped, 1),
        (RecPlay, Looping, Hold, Looping, 1),
        (RecPlay, Looping, Stop, Stopped, 1),
        (RecPlay, Looping, StopHold, Idle, 0),
        (RecPlay, Overdubbing, Press, Looping, 2),
        (RecPlay, Overdubbing, Double, Stopped, 2),
        (RecPlay, Overdubbing, Hold, Looping, 1),
        (RecPlay, Overdubbing, Stop, Stopped, 2),
        (RecPlay, Overdubbing, StopHold, Idle, 0),
        (RecPlay, Stopped, Press, Looping, 1),
        (RecPlay, Stopped, Double, Stopped, 1),
        (RecPlay, Stopped, Hold, Stopped, 1),
        (RecPlay, Stopped, Stop, Stopped, 1),
        (RecPlay, Stopped, StopHold, Idle, 0),
        // REC→DUB: a press while dubbing dubs the next layer, S steps
        // down to playing.
        (RecDub, Idle, Press, Recording, 0),
        (RecDub, Idle, Double, Idle, 0),
        (RecDub, Idle, Hold, Idle, 0),
        (RecDub, Idle, Stop, Idle, 0),
        (RecDub, Idle, StopHold, Idle, 0),
        (RecDub, Recording, Press, Overdubbing, 1),
        (RecDub, Recording, Double, Stopped, 1),
        (RecDub, Recording, Hold, Idle, 0),
        (RecDub, Recording, Stop, Stopped, 1),
        (RecDub, Recording, StopHold, Idle, 0),
        (RecDub, Overdubbing, Press, Overdubbing, 2),
        (RecDub, Overdubbing, Double, Looping, 2),
        (RecDub, Overdubbing, Hold, Looping, 1),
        (RecDub, Overdubbing, Stop, Looping, 2),
        (RecDub, Overdubbing, StopHold, Idle, 0),
        (RecDub, Looping, Press, Overdubbing, 2),
        (RecDub, Looping, Double, Stopped, 2),
        (RecDub, Looping, Hold, Looping, 1),
        (RecDub, Looping, Stop, Stopped, 2),
        (RecDub, Looping, StopHold, Idle, 0),
        (RecDub, Stopped, Press, Looping, 2),
        (RecDub, Stopped, Double, Stopped, 2),
        (RecDub, Stopped, Hold, Stopped, 1),
        (RecDub, Stopped, Stop, Stopped, 2),
        (RecDub, Stopped, StopHold, Idle, 0),
        // A layer per pass: like REC→PLAY once the take has gone into
        // its dub.
        (RecDubPerPass, Idle, Press, Recording, 0),
        (RecDubPerPass, Idle, Double, Idle, 0),
        (RecDubPerPass, Idle, Hold, Idle, 0),
        (RecDubPerPass, Idle, Stop, Idle, 0),
        (RecDubPerPass, Idle, StopHold, Idle, 0),
        (RecDubPerPass, Recording, Press, Overdubbing, 1),
        (RecDubPerPass, Recording, Double, Stopped, 1),
        (RecDubPerPass, Recording, Hold, Idle, 0),
        (RecDubPerPass, Recording, Stop, Stopped, 1),
        (RecDubPerPass, Recording, StopHold, Idle, 0),
        (RecDubPerPass, Overdubbing, Press, Looping, 2),
        (RecDubPerPass, Overdubbing, Double, Stopped, 2),
        (RecDubPerPass, Overdubbing, Hold, Looping, 1),
        (RecDubPerPass, Overdubbing, Stop, Stopped, 2),
        (RecDubPerPass, Overdubbing, StopHold, Idle, 0),
        (RecDubPerPass, Looping, Press, Overdubbing, 2),
        (RecDubPerPass, Looping, Double, Stopped, 2),
        (RecDubPerPass, Looping, Hold, Looping, 1),
        (RecDubPerPass, Looping, Stop, Stopped, 2),
        (RecDubPerPass, Looping, StopHold, Idle, 0),
        (RecDubPerPass, Stopped, Press, Looping, 2),
        (RecDubPerPass, Stopped, Double, Stopped, 2),
        (RecDubPerPass, Stopped, Hold, Stopped, 1),
        (RecDubPerPass, Stopped, Stop, Stopped, 2),
        (RecDubPerPass, Stopped, StopHold, Idle, 0),
    ];

    assert_eq!(expected.len(), setups.len() * 5, "a row for every case");

    let mut wrong = Vec::new();
    for &(mode, from, setup) in &setups {
        for gesture in [Press, Double, Hold, Stop, StopHold] {
            let config = AppConfig {
                record_mode: mode,
                ..rec_play(8)
            };
            let name = format!("looper-table-{mode:?}-{from:?}-{gesture:?}");
            let (mut looper, driver, dir) = start(&name, &config);
            let mut at = Instant::now();
            for &step in setup {
                perform(&mut looper, &driver, &mut at, step);
            }
            assert_eq!(looper.state(), from, "{mode:?} setup for {from:?}");

            perform(&mut looper, &driver, &mut at, gesture);
            let got = (looper.state(), looper.layer_count());
            let want = expected
                .iter()
                .find(|row| (row.0, row.1, row.2) == (mode, from, gesture))
                .map(|row| (row.3, row.4));
            if want != Some(got) {
                wrong.push(format!(
                    "        ({mode:?}, {from:?}, {gesture:?}, {:?}, {}),",
                    got.0, got.1
                ));
            }
            drop(looper);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    assert!(
        wrong.is_empty(),
        "rows that don't match:\n{}",
        wrong.join("\n")
    );
}

// --- What the screen asks for ---------------------------------------------

#[test]
fn the_screens_actions_reach_the_looper() {
    let (mut looper, driver, dir, at) = two_layers("looper-act");

    looper.act(LooperAction::ToggleMetronome);
    assert!(looper.is_metronome_on());
    looper.act(LooperAction::ToggleLayerMute(1));
    assert!(looper.is_layer_muted(1));
    looper.act(LooperAction::LoopVolume(40));
    looper.act(LooperAction::ClickVolume(60));
    assert_eq!(
        (looper.loop_volume_pct(), looper.click_volume_pct()),
        (40, 60)
    );

    looper.act(LooperAction::Undo);
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);
    assert_eq!(looper.layer_count(), 1, "undone");
    looper.act(LooperAction::Redo);
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);
    assert_eq!(looper.layer_count(), 2, "and redone");

    // The round button, latched like Space: read by the next tick.
    looper.act(LooperAction::ButtonHeld(true));
    looper.tick(false, at + Duration::from_secs(1));
    looper.act(LooperAction::ButtonHeld(false));
    looper.tick(false, at + Duration::from_secs(1));
    assert_eq!(looper.state(), LoopState::Overdubbing, "a press dubs");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_tempo_actions_reach_the_looper() {
    let (mut looper, _driver, dir) = start("looper-act-tempo", &settings());
    let bpm = looper.tempo();

    looper.act(LooperAction::NudgeTempo(1));
    assert_eq!(looper.tempo(), bpm + 1.0);
    let at = Instant::now();
    looper.act(LooperAction::TapTempo(at));
    looper.act(LooperAction::TapTempo(at + Duration::from_millis(500)));
    assert_eq!(looper.tempo(), 120.0, "two taps half a second apart");
    looper.act(LooperAction::ToggleMode);
    assert_eq!(looper.mode(), Mode::Extended);
    let _ = std::fs::remove_dir_all(&dir);
}
