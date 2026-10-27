use super::*;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use looper_hal::Backends;
use looper_hal::mock::{mock, split};

/// What the shell holds for this screen: the state, the backends an edit
/// is checked against - applied as `App::apply` would - and what Start
/// asked to start.
struct Shell {
    settings: SettingsState,
    backends: Backends,
    started: Option<AppConfig>,
}

fn harness(backends: Backends) -> Harness<'static, Shell> {
    let mut settings = SettingsState::new(None);
    // Not the saved config of whoever runs the tests.
    settings.config = AppConfig::default();
    settings.refresh(&backends);
    let shell = Shell {
        settings,
        backends,
        started: None,
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(320.0, 1_200.0))
        .build_ui_state(
            |ui, shell: &mut Shell| {
                for action in render(ui, &shell.settings) {
                    match action {
                        Action::EditSettings(config) => {
                            shell.settings.edit(config, &shell.backends)
                        }
                        Action::Start(config) => shell.started = Some(config),
                        Action::OpenSettings | Action::Looper(_) => {}
                    }
                }
            },
            shell,
        );
    harness.run();
    harness
}

fn duplex() -> Harness<'static, Shell> {
    harness(Backends::new(vec![Box::new(mock().0)]))
}

/// Anything with this in its label: a slider names itself twice, so
/// "exactly one" would be the wrong question.
fn shows(harness: &Harness<'_, Shell>, label: &str) -> bool {
    harness.query_all_by_label_contains(label).next().is_some()
}

#[test]
fn every_setting_is_on_the_screen() {
    // Test plan S1, S11-S13.
    let harness = duplex();
    for label in [
        "Device",
        "Sample rate",
        "Input channel",
        "Record start",
        "Stop",
        "Hold to undo / clear",
        "Metronome",
        "Time signature",
        "Record mode",
        "Latency",
        "Max loop length",
        "Max layers",
        "Reserves",
        "Start",
    ] {
        assert!(shows(&harness, label), "{label}");
    }
}

#[test]
fn one_driver_needs_no_driver_row() {
    // Test plan S2.
    assert!(!shows(&duplex(), "Driver"));
}

#[test]
fn a_split_backend_names_both_ends_and_warns() {
    // Test plan S4.
    let harness = harness(Backends::new(vec![Box::new(split().0)]));
    assert!(shows(&harness, "Input"));
    assert!(shows(&harness, "Output"));
    assert!(shows(&harness, "takes land up"));
    assert!(!shows(&harness, "Device"));
}

#[test]
fn each_record_start_shows_only_its_own_setting() {
    // Test plan S8.
    let mut harness = duplex();
    assert!(shows(&harness, "Record delay"), "Delay is the default");
    assert!(!shows(&harness, "Start level"));

    harness.get_by_label("When you play").click();
    harness.run();
    assert_eq!(
        harness.state().settings.config.record_start,
        RecordStart::Detect
    );
    assert!(shows(&harness, "Start level"));
    assert!(!shows(&harness, "Record delay"));

    harness.get_by_label("Instant").click();
    harness.run();
    assert!(!shows(&harness, "Start level") && !shows(&harness, "Record delay"));
}

#[test]
fn fade_out_shows_its_time() {
    // Test plan S9.
    let mut harness = duplex();
    assert!(!shows(&harness, "Fade time"));

    harness.get_by_label("Fade out").click();
    harness.run();
    assert!(shows(&harness, "Fade time"));
}

#[test]
fn start_starts_what_is_on_screen() {
    // Test plan S14: a change made, then Start.
    let mut harness = duplex();
    harness.get_by_label("REC > DUB").click();
    harness.run();

    harness.get_by_label("Start").click();
    harness.run();

    let started = harness.state().started.clone().expect("Start asked");
    assert_eq!(started.record_mode, RecordMode::RecDub);
}
