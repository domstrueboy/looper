use super::*;

use std::path::PathBuf;
use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use looper_core::config::{AppConfig, RecordStart};
use looper_hal::mock::{self, MockDriver, mock};
use looper_hal::{BackendId, Backends};

/// What the shell holds: the looper, and what the screen asked of it -
/// applied as `App::apply` would, and kept so a test can say what was
/// asked.
///
/// The screen restates both button latches every frame, so a release
/// can't be missed; kept here only when one moves, since that's what a
/// test is asking about.
struct Shell {
    looper: LooperState,
    asked: Vec<LooperAction>,
    latches: (bool, bool),
    opened_settings: bool,
}

impl Shell {
    fn take(&mut self, action: LooperAction) {
        let moved = match action {
            LooperAction::ButtonHeld(held) => std::mem::replace(&mut self.latches.0, held) != held,
            LooperAction::StopButtonHeld(held) => {
                std::mem::replace(&mut self.latches.1, held) != held
            }
            _ => true,
        };
        if moved {
            self.asked.push(action);
        }
        self.looper.act(action);
    }
}

/// A looper on the mock device, its screen at the window's size, and
/// the clock its presses are timed on.
struct Rig {
    harness: Harness<'static, Shell>,
    driver: MockDriver,
    dir: PathBuf,
    at: Instant,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn settings() -> AppConfig {
    AppConfig {
        backend: BackendId::MOCK.as_str().to_string(),
        device_name: mock::DEVICE.to_string(),
        sample_rate: 44_100,
        record_start: RecordStart::Instant,
        preroll_ms: 0,
        long_press_ms: 500,
        ..AppConfig::default()
    }
}

fn rig(name: &str, config: AppConfig) -> Rig {
    let (backend, driver) = mock();
    let backends = Backends::new(vec![Box::new(backend)]);
    let dir = std::env::temp_dir().join(format!("looper-pedal-ui-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let looper = LooperState::start_in(&backends, &config, dir.clone()).expect("the mock opens");
    let shell = Shell {
        looper,
        asked: Vec::new(),
        latches: (false, false),
        opened_settings: false,
    };
    let harness = Harness::builder()
        .with_size(egui::vec2(320.0, 720.0))
        .build_ui_state(
            |ui, shell: &mut Shell| {
                for action in render(ui, &shell.looper) {
                    match action {
                        Action::Looper(action) => shell.take(action),
                        Action::OpenSettings => shell.opened_settings = true,
                        Action::Start(_) | Action::EditSettings(_) => {}
                    }
                }
            },
            shell,
        );
    Rig {
        harness,
        driver,
        dir,
        at: Instant::now(),
    }
}

impl Rig {
    fn looper(&mut self) -> &mut LooperState {
        &mut self.harness.state_mut().looper
    }

    /// Space, down and up, a second after the last - never a double.
    fn press(&mut self) {
        self.at += Duration::from_secs(1);
        let at = self.at;
        self.looper().tick(true, at);
        self.looper().tick(false, at + Duration::from_millis(50));
        self.feed(4_096);
    }

    /// S, the same way.
    fn stop(&mut self) {
        self.at += Duration::from_secs(1);
        let at = self.at;
        self.looper().set_stop_key_held(true);
        self.looper().tick(false, at);
        self.looper().set_stop_key_held(false);
        self.looper().tick(false, at + Duration::from_millis(50));
        self.feed(4_096);
    }

    /// Audio through the mock, then a tick and a frame - what the next
    /// frames would show once the audio thread has caught up.
    fn feed(&mut self, frames: usize) {
        for _ in 0..frames.div_ceil(4_096) {
            self.driver.cycle(&vec![0.25; 4_096]);
            let at = self.at;
            self.looper().tick(false, at);
        }
        self.harness.run();
    }

    /// A first take, ended into playing.
    fn record(&mut self) {
        self.press();
        self.feed(16_384);
        self.press();
    }

    fn asked(&mut self) -> Vec<LooperAction> {
        std::mem::take(&mut self.harness.state_mut().asked)
    }

    fn shows(&self, label: &str) -> bool {
        self.harness.query_by_label(label).is_some()
    }

    fn enabled(&self, label: &str) -> bool {
        !self
            .harness
            .get_by_label(label)
            .accesskit_node()
            .is_disabled()
    }

    fn click(&mut self, label: &str) {
        self.harness.get_by_label(label).click();
        self.harness.run();
    }

    fn key(&mut self, key: egui::Key) {
        self.harness.key_press(key);
        self.harness.run();
    }
}

#[test]
fn an_empty_looper_offers_only_recording() {
    // Test plan L1.
    let mut rig = rig("empty", settings());
    rig.harness.run();

    assert!(rig.shows("Empty"));
    assert!(rig.shows("⏺"), "the round button records");
    assert!(rig.shows("Layers: 0/8"));
    assert!(!rig.enabled("Stop (S)"), "nothing to stop or clear");
    assert!(!rig.enabled("Undo"));
    assert!(!rig.enabled("Redo (R)"));
    assert!(rig.shows("Space or button: record / play / dub"));
}

#[test]
fn each_state_shows_its_label_and_what_a_press_does_next() {
    // Test plan L3-L8.
    let mut rig = rig("states", settings());
    rig.press();
    assert!(rig.shows("Recording") && rig.shows("⏹"));

    rig.feed(16_384);
    rig.press();
    assert!(rig.shows("Looping") && rig.shows("⏺"));
    assert!(rig.shows("Layers: 1/8"));

    rig.press();
    assert!(rig.shows("Overdubbing") && rig.shows("▶"));

    rig.feed(4_096);
    rig.press();
    assert!(rig.shows("Looping") && rig.shows("Layers: 2/8"));

    rig.stop();
    assert!(rig.shows("Stopped") && rig.shows("▶"));
}

#[test]
fn undo_and_redo_ask_for_themselves_and_follow_the_stack() {
    // Test plan L9-L10.
    let mut rig = rig("undo-redo", settings());
    rig.record();
    rig.press();
    rig.feed(4_096);
    rig.press();
    assert!(rig.enabled("Undo"));
    rig.asked();

    rig.click("Undo");
    assert_eq!(rig.asked(), vec![LooperAction::Undo]);
    rig.feed(4_096);
    assert!(rig.shows("Layers: 1/8"));
    assert!(!rig.enabled("Undo"), "never the first take");
    assert!(rig.enabled("Redo (R)"));

    rig.key(egui::Key::R);
    assert_eq!(rig.asked(), vec![LooperAction::Redo]);
    rig.feed(4_096);
    assert!(rig.shows("Layers: 2/8"));
}

#[test]
fn the_round_button_is_latched_like_space() {
    // Down on the button is a press for the next tick, up a release.
    let mut rig = rig("button", settings());
    rig.harness.run();

    rig.click("⏺");
    assert_eq!(
        rig.asked(),
        vec![
            LooperAction::ButtonHeld(true),
            LooperAction::ButtonHeld(false)
        ]
    );
}

#[test]
fn stop_s_hold_label_and_gear() {
    let mut rig = rig("stop-gear", settings());
    rig.record();
    assert!(rig.enabled("Stop (S)"));

    rig.click("⚙");
    assert!(rig.harness.state().opened_settings);
}

#[test]
fn the_mode_switch_waits_for_the_loop_to_stop() {
    // Test plan L15.
    let mut rig = rig("mode", settings());
    rig.record();
    assert!(!rig.enabled("Extended"), "locked while it plays");

    rig.stop();
    assert!(rig.enabled("Extended"));
    rig.click("Extended");
    assert_eq!(rig.asked(), vec![LooperAction::ToggleMode]);
    assert_eq!(rig.harness.state().looper.mode(), Mode::Extended);
}

#[test]
fn the_metronome_keys_ask_for_what_their_buttons_do() {
    // Test plan L16-L18.
    let mut rig = rig("metronome", settings());
    rig.harness.run();

    rig.key(egui::Key::M);
    rig.key(egui::Key::Comma);
    rig.key(egui::Key::Period);
    rig.click("+");
    rig.click("−");
    let asked = rig.asked();
    assert_eq!(
        asked,
        vec![
            LooperAction::ToggleMetronome,
            LooperAction::NudgeTempo(-1),
            LooperAction::NudgeTempo(1),
            LooperAction::NudgeTempo(1),
            LooperAction::NudgeTempo(-1),
        ]
    );
    assert!(rig.harness.state().looper.is_metronome_on());
    assert!(rig.shows("Metronome 120 BPM (M)"));

    rig.key(egui::Key::T);
    assert!(matches!(rig.asked()[..], [LooperAction::TapTempo(_)]));
}

#[test]
fn a_loops_tempo_turns_the_steps_into_halve_and_double() {
    // Test plan L19.
    let mut rig = rig("loop-tempo", settings());
    rig.record();
    rig.feed(4_096);

    assert!(rig.shows("½") && rig.shows("2×"));
    assert!(!rig.shows("+"));
    assert!(!rig.enabled("Tap (T)"), "the loop sets the tempo");
    assert!(
        rig.harness
            .query_by_label_contains("Metronome")
            .and_then(|node| node.accesskit_node().label())
            .is_some_and(|label| label.contains('.')),
        "shown to a tenth"
    );
}

#[test]
fn extended_mutes_a_layer_by_button_or_number_key() {
    // Test plan E1-E3.
    let config = AppConfig {
        mode: Mode::Extended,
        ..settings()
    };
    let mut rig = rig("extended", config);
    rig.harness.run();
    assert!(rig.shows("Layers appear here once there's a loop"));

    rig.record();
    rig.press();
    rig.feed(4_096);
    rig.press();
    assert!(rig.shows("1-2: mute / unmute a layer"));
    rig.asked();

    rig.click("2");
    rig.key(egui::Key::Num1);
    assert_eq!(
        rig.asked(),
        vec![
            LooperAction::ToggleLayerMute(1),
            LooperAction::ToggleLayerMute(0)
        ]
    );
    let looper = &rig.harness.state().looper;
    assert!(looper.is_layer_muted(0) && looper.is_layer_muted(1));
}

#[test]
fn page_down_and_page_up_are_the_two_pedals() {
    // What fixed-key foot pedals send, read as Space and S are.
    let mut harness = Harness::new_ui_state(
        |ui, held: &mut (bool, bool)| *held = pedals_held(ui.ctx()),
        (false, false),
    );
    for (key, expected) in [
        (egui::Key::Space, (true, false)),
        (egui::Key::PageDown, (true, false)),
        (egui::Key::S, (false, true)),
        (egui::Key::PageUp, (false, true)),
    ] {
        harness.key_down(key);
        harness.run();
        assert_eq!(*harness.state(), expected, "{key:?} down");
        harness.key_up(key);
        harness.run();
        assert_eq!(*harness.state(), (false, false), "{key:?} up");
    }
}
