use looper_core::action::Action;
use looper_core::config::{
    AfterTake, LATENCY_MS_RANGE, LONG_PRESS_MS_RANGE, MAX_LAYERS_RANGE, MAX_LOOP_SECS_RANGE,
    METRONOME_BPM_RANGE, PREROLL_MS_RANGE, VOLUME_PCT_RANGE,
};
use looper_core::settings::SettingsState;
use looper_hal::Backends;

/// Reserved below the scrolling list so Start is always reachable.
const START_ROW_HEIGHT: f32 = 40.0;

pub fn render(
    ui: &mut egui::Ui,
    settings: &mut SettingsState,
    backends: &Backends,
) -> Option<Action> {
    let mut action = None;

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
            // up below the fold.
            let can_start = egui::ScrollArea::vertical()
                .max_height((ui.available_height() - START_ROW_HEIGHT).max(120.0))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| controls(ui, settings, backends)).inner
                })
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
                    action = Some(Action::Start(settings.config.clone()));
                }
            });
        });

    action
}

/// The pickers and sliders. Returns whether there's a startable choice:
/// a device that reports no usable rate or no inputs can't be opened, and
/// says so instead of offering Start.
fn controls(ui: &mut egui::Ui, settings: &mut SettingsState, backends: &Backends) -> bool {
    if settings.backends.is_empty() {
        ui.colored_label(egui::Color32::RED, "No audio backends in this build.");
        return false;
    }

    // Only worth choosing when there is a choice - one backend picks
    // itself, and a row saying so is a row in the way.
    if settings.backends.len() > 1 {
        let previous = settings.config.backend.clone();
        let selected = settings
            .backends
            .iter()
            .find(|backend| backend.id == settings.config.backend)
            .map(|backend| backend.label.clone())
            .unwrap_or_default();
        egui::ComboBox::from_label("Driver")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for backend in &settings.backends {
                    ui.selectable_value(
                        &mut settings.config.backend,
                        backend.id.clone(),
                        &backend.label,
                    );
                }
            });
        if settings.config.backend != previous {
            settings.refresh_for_selected_backend(backends);
        }
    }

    if settings.inputs.is_empty() {
        ui.colored_label(egui::Color32::RED, "This driver reports no input devices.");
        return false;
    }

    let previous_device = settings.config.device_name.clone();
    // "Device" while one does both, "Input" once they are separate -
    // naming it Input when there is nothing else to pick reads as though
    // an output picker had gone missing.
    egui::ComboBox::from_label(if settings.duplex { "Device" } else { "Input" })
        .selected_text(settings.config.device_name.clone())
        .show_ui(ui, |ui| {
            for name in &settings.inputs {
                ui.selectable_value(&mut settings.config.device_name, name.clone(), name);
            }
        });
    if settings.config.device_name != previous_device {
        settings.refresh_for_selected_device(backends);
    }

    // A backend whose devices are single endpoints needs both ends
    // naming. Left out entirely when one device does both, rather than
    // shown greyed: there is nothing there to choose.
    if !settings.duplex {
        let previous_output = settings.config.output_device().to_string();
        let mut chosen = previous_output.clone();
        egui::ComboBox::from_label("Output")
            .selected_text(chosen.clone())
            .show_ui(ui, |ui| {
                for name in &settings.outputs {
                    ui.selectable_value(&mut chosen, name.clone(), name);
                }
            });
        if chosen != previous_output {
            settings.config.output_device_name = Some(chosen);
            settings.refresh_for_selected_device(backends);
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
        ui.colored_label(
            egui::Color32::RED,
            "No sample rate both ends agree on.",
        );
        return false;
    }

    egui::ComboBox::from_label("Sample rate")
        .selected_text(format!("{} Hz", settings.config.sample_rate))
        .show_ui(ui, |ui| {
            for &rate in &settings.sample_rates {
                ui.selectable_value(&mut settings.config.sample_rate, rate, format!("{rate} Hz"));
            }
        });

    if settings.input_channels == 0 {
        ui.colored_label(egui::Color32::RED, "This device reports no input channels.");
        return false;
    }

    egui::ComboBox::from_label("Input channel")
        .selected_text(format!("Input {}", settings.config.input_channel + 1))
        .show_ui(ui, |ui| {
            for channel in 0..settings.input_channels {
                ui.selectable_value(
                    &mut settings.config.input_channel,
                    channel,
                    format!("Input {}", channel + 1),
                );
            }
        });

    ui.add_space(8.0);
    ui.separator();

    ui.add(
        egui::Slider::new(&mut settings.config.volume_pct, VOLUME_PCT_RANGE)
            .text("Loop volume")
            .suffix("%"),
    );
    ui.add(
        egui::Slider::new(&mut settings.config.preroll_ms, PREROLL_MS_RANGE)
            .step_by(500.0)
            .text("Record delay")
            // Shown in seconds, and as "off" rather than "0.0 s" so the
            // choice reads as deliberate.
            .custom_formatter(|ms, _| {
                if ms < 1.0 {
                    "off".to_owned()
                } else {
                    format!("{:.1} s", ms / 1000.0)
                }
            }),
    );
    ui.add(
        egui::Slider::new(&mut settings.config.long_press_ms, LONG_PRESS_MS_RANGE)
            .step_by(250.0)
            .text("Hold to clear")
            .custom_formatter(|ms, _| format!("{:.2} s", ms / 1000.0)),
    );
    ui.add(
        egui::Slider::new(&mut settings.config.metronome_bpm, METRONOME_BPM_RANGE)
            .text("Metronome")
            .suffix(" BPM"),
    );
    // What the press that ends the first take does next. Overdub is the
    // NUX's REC -> DUB -> PLAY: the second part goes down straight away.
    ui.horizontal(|ui| {
        ui.label("After recording");
        ui.radio_value(&mut settings.config.after_take, AfterTake::Play, "Play");
        ui.radio_value(&mut settings.config.after_take, AfterTake::Dub, "Overdub");
    });

    ui.add_space(8.0);
    ui.separator();

    ui.add(
        egui::Slider::new(&mut settings.config.latency_ms, LATENCY_MS_RANGE)
            .text("Latency")
            .suffix(" ms"),
    );
    ui.add(
        egui::Slider::new(&mut settings.config.max_loop_secs, MAX_LOOP_SECS_RANGE)
            .step_by(10.0)
            .text("Max loop length")
            .suffix(" s"),
    );
    ui.add(egui::Slider::new(&mut settings.config.max_layers, MAX_LAYERS_RANGE).text("Max layers"));
    // Every layer is pre-allocated at the full loop length, so these two
    // settings have a price worth seeing before it's paid.
    ui.label(format!(
        "Reserves {} MB of memory",
        settings.config.loop_memory_bytes() / (1024 * 1024)
    ));

    true
}
