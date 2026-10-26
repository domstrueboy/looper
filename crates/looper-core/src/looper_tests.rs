use super::*;

use looper_hal::{BackendId, HalError};
use looper_hal::mock::{self, MockDriver, mock};

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

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("looper-pedal-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
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

/// One press: down on a frame, up on the next. `ShortPress` is a release,
/// so it takes both.
fn press(looper: &mut LooperState, down: Instant, up: Instant) {
    looper.tick(true, down);
    looper.tick(false, up);
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

    // Down before the countdown runs out, up exactly as it does.
    let expiry = start + Duration::from_millis(PREROLL_MS as u64);
    looper.tick(true, expiry - Duration::from_millis(1));
    looper.tick(false, expiry);

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
    let mut step = |looper: &mut LooperState| {
        at += Duration::from_millis(10);
        press(looper, at, at);
    };

    step(&mut looper);
    assert_eq!(looper.state(), LoopState::Recording, "no pre-roll, straight in");

    driver.cycle(&vec![0.5; 2_048]);
    step(&mut looper);
    assert_eq!(looper.state(), LoopState::Looping);

    // The audio thread needs a callback to publish what it settled on.
    driver.cycle(&vec![0.0; 512]);
    at += Duration::from_millis(10);
    looper.tick(false, at);
    assert_eq!(looper.layer_count(), 1, "one layer, from one take");

    // Held past the threshold: a long press clears rather than cycling.
    looper.tick(true, at);
    looper.tick(true, at + Duration::from_millis(600));
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

    let reported = looper.device_fault().expect("the screen has something to show");
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
    assert_eq!(saved.mode, Mode::Extended, "the app reopens the way it was left");
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
    let at = Instant::now();
    press(&mut looper, at, at);
    driver.cycle(&vec![0.5; 2_048]);
    press(&mut looper, at, at);
    driver.cycle(&vec![0.0; 512]);

    // Ticked between the steps, as frames would: the UI thread's copy of
    // the loop only sees a take it's ticked through.
    looper.toggle_overdub();
    looper.tick(false, at);
    driver.cycle(&vec![0.25; 512]);
    looper.toggle_overdub();
    looper.tick(false, at);
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
    assert!(looper.is_layer_muted(0), "the layer still there keeps its mute");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clearing_forgets_every_mute() {
    let (mut looper, _driver, dir, at) = two_layers("looper-mute-clear");
    looper.toggle_layer_mute(0);

    looper.tick(true, at);
    looper.tick(true, at + Duration::from_millis(600));
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
    assert!(
        config::loop_dir_in(&dir).join("layer-2.wav").exists(),
        "and on disk"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_overdub_after_a_remove_ends_the_chance_to_redo() {
    let (mut looper, driver, dir, at) = two_layers("looper-redo-overdub");
    looper.remove_last_layer();
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);

    looper.toggle_overdub();
    looper.tick(false, at);
    driver.cycle(&vec![0.25; 512]);
    looper.toggle_overdub();
    looper.tick(false, at);

    assert!(!looper.can_redo_layer());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn removing_the_only_layer_goes_idle_and_redo_brings_it_back_stopped() {
    let no_preroll = AppConfig {
        preroll_ms: 0,
        ..settings()
    };
    let (mut looper, driver, dir) = start("looper-redo-only", &no_preroll);
    let at = Instant::now();
    press(&mut looper, at, at);
    driver.cycle(&vec![0.5; 2_048]);
    press(&mut looper, at, at);
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);

    looper.remove_last_layer();
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);
    assert_eq!(looper.state(), LoopState::Idle);
    assert_eq!(looper.layer_count(), 0);

    looper.redo_layer();
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);

    assert_eq!(looper.state(), LoopState::Stopped, "there, but silent");
    assert_eq!(looper.layer_count(), 1);
    assert!(config::loop_dir_in(&dir).join("layer-1.wav").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_long_press_clear_is_final() {
    let (mut looper, _driver, dir, at) = two_layers("looper-redo-clear");
    looper.remove_last_layer();

    looper.tick(true, at);
    looper.tick(true, at + Duration::from_millis(600));
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
fn the_metronome_counts_in_through_the_pre_roll() {
    let (mut looper, driver, dir) = start("looper-metronome-arming", &settings());
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
    assert_eq!(saved.metronome_bpm, 300, "Settings and the next start agree");

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
fn record_against_the_click(name: &str, frames: usize) -> (LooperState, MockDriver, PathBuf, Instant) {
    let (mut looper, driver, dir) = start(name, &settings());
    looper.toggle_metronome();
    let at = Instant::now();
    press(&mut looper, at, at);
    feed(&mut looper, &driver, BAR, at);
    feed(&mut looper, &driver, frames, at);
    assert_eq!(looper.state(), LoopState::Recording, "counted in");
    press(&mut looper, at, at);
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
    let (mut looper, driver, dir, at) = record_against_the_click("looper-bars-late", 3 * BAR + BAR / 3);
    assert_eq!(looper.state(), LoopState::Looping, "stops straight away");
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);

    let (secs, _) = looper.loop_duration_and_progress();
    assert_eq!(secs, 6.0, "three bars of 2 s");
    let (saved, _) = crate::wav::read(&config::loop_dir_in(&dir).join("layer-1.wav"))
        .expect("saved");
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
    let (mut looper, _driver, dir, at) =
        record_against_the_click("looper-bars-now", 3 * BAR + 2 * BAR / 3);
    press(&mut looper, at, at);
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
    let at = Instant::now();
    press(&mut looper, at, at);
    feed(&mut looper, &driver, BAR + 1_000, at);
    press(&mut looper, at, at);
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);

    let (secs, _) = looper.loop_duration_and_progress();
    assert!((secs - (BAR + 1_000) as f32 / 44_100.0).abs() < 0.02, "got {secs}");
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Count-in ---------------------------------------------------------------

#[test]
fn with_the_metronome_on_a_press_counts_in_one_bar_not_the_record_delay() {
    // The configured delay is a second; the bar is two.
    let (mut looper, driver, dir) = start("looper-count-in", &settings());
    looper.toggle_metronome();
    let at = Instant::now();
    press(&mut looper, at, at);
    assert_eq!(looper.state(), LoopState::Arming);

    // Past the record delay, well short of the bar: still counting,
    // however much time the frames say has gone by.
    feed(&mut looper, &driver, BAR / 2, at);
    looper.tick(false, at + Duration::from_millis(PREROLL_MS as u64 + 100));
    assert_eq!(looper.state(), LoopState::Arming);
    assert!((looper.preroll_remaining_secs() - 0.9).abs() < 0.01, "a bar of 2 s");

    feed(&mut looper, &driver, BAR / 2 + 1_000, at);
    assert_eq!(looper.state(), LoopState::Recording, "on the bar line");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn turning_the_metronome_off_calls_a_count_in_off() {
    let (mut looper, driver, dir) = start("looper-count-in-off", &settings());
    looper.toggle_metronome();
    let at = Instant::now();
    press(&mut looper, at, at);

    looper.toggle_metronome();

    assert_eq!(looper.state(), LoopState::Idle);
    feed(&mut looper, &driver, 2 * BAR, at);
    assert_eq!(looper.state(), LoopState::Idle, "and nothing starts later");
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
    let at = Instant::now();
    press(&mut looper, at, at);
    feed(&mut looper, &driver, frames, at);
    press(&mut looper, at, at);
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);
    (looper, driver, dir, at)
}

#[test]
fn a_loop_played_without_the_click_gets_a_tempo_from_its_length() {
    // 2.6 s is four beats at 92.3.
    let (looper, _driver, dir, _) = free_take("looper-tempo-guess", 114_660);
    assert!(looper.tempo_is_the_loops());
    assert!((looper.tempo() - 92.3).abs() < 0.1, "got {}", looper.tempo());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_loop_played_to_the_click_keeps_the_clicks_tempo() {
    // Three bars at 120: the guess would have made it 160 or 80.
    let (mut looper, driver, dir, at) = record_against_the_click("looper-tempo-click", 3 * BAR);
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);
    assert!(looper.tempo_is_the_loops());
    assert!((looper.tempo() - 120.0).abs() < 0.01, "got {}", looper.tempo());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wrong_guess_is_halved_or_doubled_but_not_nudged() {
    let (mut looper, _driver, dir, _) = free_take("looper-tempo-fix", 88_200);
    assert!((looper.tempo() - 120.0).abs() < 0.01, "two seconds, four beats");

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
    looper.tick(true, at);
    looper.tick(true, at + Duration::from_millis(600));
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
    assert!((restored.tempo() - 120.0).abs() < 0.01, "got {}", restored.tempo());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_halved_tempo_survives_a_restart() {
    // Guessed at 120, halved to 60: a restart must not guess 120 again.
    let (mut looper, _driver, dir, _) = free_take("looper-tempo-saved", 88_200);
    looper.halve_tempo();
    drop(looper);

    let restored = reopen(&dir);

    assert!((restored.tempo() - 60.0).abs() < 0.01, "got {}", restored.tempo());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_loop_played_to_the_click_comes_back_at_the_clicks_tempo() {
    // Three bars at 120 is six seconds - which a guess would call 80.
    let (mut looper, driver, dir, at) = record_against_the_click("looper-tempo-click-saved", 3 * BAR);
    driver.cycle(&vec![0.0; 512]);
    looper.tick(false, at);
    drop(looper);

    let restored = reopen(&dir);

    assert!((restored.tempo() - 120.0).abs() < 0.01, "got {}", restored.tempo());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_saved_tempo_the_click_cannot_play_is_guessed_instead() {
    let (looper, _driver, dir, _) = free_take("looper-tempo-saved-bad", 88_200);
    drop(looper);
    // 1,000 beats in two seconds is 30,000 BPM.
    loop_mirror::save_beats(&config::loop_dir_in(&dir), 1_000).unwrap();

    let restored = reopen(&dir);

    assert!((restored.tempo() - 120.0).abs() < 0.01, "got {}", restored.tempo());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clearing_deletes_the_saved_tempo() {
    let (mut looper, _driver, dir, at) = free_take("looper-tempo-saved-clear", 88_200);
    assert!(loop_mirror::load_beats(&config::loop_dir_in(&dir)).is_some(), "saved");

    looper.tick(true, at);
    looper.tick(true, at + Duration::from_millis(600));

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
