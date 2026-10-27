use looper_core::action::Action;
use looper_core::config::AppConfig;
use looper_core::config::{
    DETECT_DB_RANGE, FADE_MS_RANGE, LATENCY_MS_RANGE, LONG_PRESS_MS_RANGE, MAX_LAYERS_RANGE,
    MAX_LOOP_SECS_RANGE, METRONOME_BPM_RANGE, PREROLL_MS_RANGE, RecordMode, RecordStart, StopMode,
    TimeSignature,
};
use looper_core::settings::SettingsState;

/// Reserved below the scrolling list so Start is always reachable.
const START_ROW_HEIGHT: f32 = 40.0;

/// Reads the settings and says what was asked of them - a leaf, like the
/// looper screen. The widgets edit a copy of the choice; a copy that
/// changed goes back as `EditSettings`, and the app decides what that
/// means for the device lists.
pub fn render(ui: &mut egui::Ui, settings: &SettingsState) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut draft = settings.config.clone();

    egui::Frame::default()
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.label("Looper Pedal - Settings");
                ui.add_space(8.0);

                if let Some(err) = &settings.error {
                    ui.colored_label(egui::Color32::RED, err.as_str());
                    ui.add_space(8.0);
                }
            });

            // More settings than the window is tall, so the list
            // scrolls - with Start kept outside it, where it can't end
            // up below the fold. Left-aligned: centred, the radio groups
            // and their section labels wandered with each line's width.
            let can_start = egui::ScrollArea::vertical()
                .max_height((ui.available_height() - START_ROW_HEIGHT).max(120.0))
                .show(ui, |ui| controls(ui, settings, &mut draft))
                .inner;

            if !can_start {
                return;
            }

            ui.add_space(6.0);
            ui.vertical_centered(|ui| {
                if ui
                    .add_sized([140.0, 26.0], egui::Button::new("Start"))
                    .clicked()
                {
                    // What's on screen, edits this frame included.
                    actions.push(Action::Start(draft.clone()));
                }
            });
        });

    if draft != settings.config {
        actions.push(Action::EditSettings(draft));
    }
    actions
}

/// The pickers and sliders. Returns whether there's a startable choice:
/// a device that reports no usable rate or no inputs can't be opened, and
/// says so instead of offering Start.
fn controls(ui: &mut egui::Ui, settings: &SettingsState, draft: &mut AppConfig) -> bool {
    if settings.backends.is_empty() {
        ui.colored_label(egui::Color32::RED, "No audio backends in this build.");
        return false;
    }

    // Only worth choosing when there is a choice - one backend picks
    // itself, and a row saying so is a row in the way.
    if settings.backends.len() > 1 {
        let selected = settings
            .backends
            .iter()
            .find(|backend| backend.id == draft.backend)
            .map(|backend| backend.label.clone())
            .unwrap_or_default();
        egui::ComboBox::from_label("Driver")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for backend in &settings.backends {
                    ui.selectable_value(&mut draft.backend, backend.id.clone(), &backend.label);
                }
            });
    }

    if settings.inputs.is_empty() {
        ui.colored_label(egui::Color32::RED, "This driver reports no input devices.");
        return false;
    }

    // "Device" while one does both, "Input" once they are separate -
    // naming it Input when there is nothing else to pick reads as though
    // an output picker had gone missing.
    egui::ComboBox::from_label(if settings.duplex { "Device" } else { "Input" })
        .selected_text(draft.device_name.clone())
        .show_ui(ui, |ui| {
            for name in &settings.inputs {
                ui.selectable_value(&mut draft.device_name, name.clone(), name);
            }
        });

    // A backend whose devices are single endpoints needs both ends
    // naming. Left out entirely when one device does both, rather than
    // shown greyed: there is nothing there to choose.
    if !settings.duplex {
        let previous_output = draft.output_device().to_string();
        let mut chosen = previous_output.clone();
        egui::ComboBox::from_label("Output")
            .selected_text(chosen.clone())
            .show_ui(ui, |ui| {
                for name in &settings.outputs {
                    ui.selectable_value(&mut chosen, name.clone(), name);
                }
            });
        if chosen != previous_output {
            draft.output_device_name = Some(chosen);
        }

        // Measured rather than guessed: against ASIO's one callback
        // period, two endpoints wander by about twenty milliseconds, and
        // the monitoring delay is fixed so nothing takes it back out.
        ui.colored_label(
            egui::Color32::from_rgb(200, 150, 60),
            "This driver runs the two halves separately, so takes land up\nto ~20 ms from where you heard them. ASIO is tighter.",
        );
    }

    if settings.sample_rates.is_empty() {
        ui.colored_label(egui::Color32::RED, "No sample rate both ends agree on.");
        return false;
    }

    egui::ComboBox::from_label("Sample rate")
        .selected_text(format!("{} Hz", draft.sample_rate))
        .show_ui(ui, |ui| {
            for &rate in &settings.sample_rates {
                ui.selectable_value(&mut draft.sample_rate, rate, format!("{rate} Hz"));
            }
        });

    if settings.input_channels == 0 {
        ui.colored_label(egui::Color32::RED, "This device reports no input channels.");
        return false;
    }

    egui::ComboBox::from_label("Input channel")
        .selected_text(format!("Input {}", draft.input_channel + 1))
        .show_ui(ui, |ui| {
            for channel in 0..settings.input_channels {
                ui.selectable_value(
                    &mut draft.input_channel,
                    channel,
                    format!("Input {}", channel + 1),
                );
            }
        });

    ui.add_space(8.0);
    ui.separator();

    // Each start's own setting sits under it, and only shows when that
    // start is picked: a delay slider beside Detect would look like it
    // does something.
    ui.label("Record start");
    ui.indent("record_start", |ui| {
        let config = &mut *draft;
        ui.radio_value(&mut config.record_start, RecordStart::Instant, "Instant")
            .on_hover_text("Recording starts as Space goes down");
        ui.radio_value(&mut config.record_start, RecordStart::Delay, "Delay")
            .on_hover_text("A countdown first, to get your hands on the guitar");
        if config.record_start == RecordStart::Delay {
            ui.add(
                egui::Slider::new(&mut config.preroll_ms, PREROLL_MS_RANGE)
                    .step_by(500.0)
                    .text("Record delay")
                    // Shown in seconds, and as "off" rather than "0.0 s"
                    // so the choice reads as deliberate.
                    .custom_formatter(|ms, _| {
                        if ms < 1.0 {
                            "off".to_owned()
                        } else {
                            format!("{:.1} s", ms / 1000.0)
                        }
                    }),
            );
        }
        ui.radio_value(&mut config.record_start, RecordStart::CountIn, "Count-in")
            .on_hover_text("A bar of clicks, then recording starts on the bar line");
        ui.radio_value(
            &mut config.record_start,
            RecordStart::Detect,
            "When you play",
        )
        .on_hover_text("Recording starts at your first note");
        if config.record_start == RecordStart::Detect {
            ui.add(
                egui::Slider::new(&mut config.detect_db, DETECT_DB_RANGE)
                    .text("Start level")
                    .suffix(" dB"),
            )
            .on_hover_text("Raise it if noise starts the recording on its own");
        }
    });
    ui.label("Stop");
    ui.indent("stop_mode", |ui| {
        let config = &mut *draft;
        ui.radio_value(&mut config.stop_mode, StopMode::Immediate, "Immediately")
            .on_hover_text("The loop goes silent the moment you stop it");
        ui.radio_value(&mut config.stop_mode, StopMode::LoopEnd, "At the loop end")
            .on_hover_text("The loop plays to its end first; stop again for now");
        ui.radio_value(&mut config.stop_mode, StopMode::FadeOut, "Fade out")
            .on_hover_text("The loop fades to silence; stop again for now");
        if config.stop_mode == StopMode::FadeOut {
            ui.add(
                egui::Slider::new(&mut config.fade_ms, FADE_MS_RANGE)
                    .step_by(500.0)
                    .text("Fade time")
                    .custom_formatter(|ms, _| format!("{:.1} s", ms / 1000.0)),
            );
        }
    });
    ui.add(
        egui::Slider::new(&mut draft.long_press_ms, LONG_PRESS_MS_RANGE)
            .step_by(250.0)
            .text("Hold to undo / clear")
            .custom_formatter(|ms, _| format!("{:.2} s", ms / 1000.0)),
    );
    ui.add(
        egui::Slider::new(&mut draft.metronome_bpm, METRONOME_BPM_RANGE)
            .text("Metronome")
            .suffix(" BPM"),
    );
    ui.horizontal(|ui| {
        ui.label("Time signature");
        let signature = &mut draft.time_signature;
        ui.radio_value(signature, TimeSignature::FourFour, "4/4");
        ui.radio_value(signature, TimeSignature::ThreeFour, "3/4");
        ui.radio_value(signature, TimeSignature::SixEight, "6/8")
            .on_hover_text("Six clicks a bar: the BPM counts eighths");
    });
    // ">" rather than an arrow: egui's bundled proportional font has no
    // arrow glyph, and it would show as an empty box.
    // One under another: three side by side don't fit the window.
    ui.label("Record mode");
    ui.indent("record_mode", |ui| {
        let mode = &mut draft.record_mode;
        ui.radio_value(mode, RecordMode::RecPlay, "REC > PLAY")
            .on_hover_text("The take plays; Space then dubs and plays in turn");
        ui.radio_value(mode, RecordMode::RecDub, "REC > DUB")
            .on_hover_text("The take goes straight into a dub; Space then dubs the next layer");
        ui.radio_value(
            mode,
            RecordMode::RecDubPerPass,
            "REC > DUB, a layer per pass",
        )
        .on_hover_text("The take goes straight into a dub; every pass is a layer of its own");
    });

    ui.add_space(8.0);
    ui.separator();

    ui.add(
        egui::Slider::new(&mut draft.latency_ms, LATENCY_MS_RANGE)
            .text("Latency")
            .suffix(" ms"),
    );
    ui.add(
        egui::Slider::new(&mut draft.max_loop_secs, MAX_LOOP_SECS_RANGE)
            .step_by(10.0)
            .text("Max loop length")
            .suffix(" s"),
    );
    ui.add(egui::Slider::new(&mut draft.max_layers, MAX_LAYERS_RANGE).text("Max layers"));
    // Every layer is pre-allocated at the full loop length, so these two
    // settings have a price worth seeing before it's paid. The copy kept
    // for saving grows to as much again as the layers fill.
    let reserved_mb = draft.loop_memory_bytes() / (1024 * 1024);
    ui.label(format!(
        "Reserves {reserved_mb} MB of memory, up to {} MB with every layer recorded",
        2 * reserved_mb
    ));

    true
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
