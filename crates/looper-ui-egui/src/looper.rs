use looper_core::action::Action;
use looper_core::config::Mode;
use looper_core::state_machine::LoopState;
use looper_core::looper::LooperState;
use crate::indicator;

/// The secondary buttons share a size, so the row can be measured and
/// centered without laying it out twice. Three of them fit the window.
const CONTROL_BUTTON_SIZE: [f32; 2] = [88.0, 24.0];

/// The metronome row: a toggle naming its tempo, between a slower and a
/// faster button.
const METRONOME_BUTTON_SIZE: [f32; 2] = [170.0, 24.0];
const TEMPO_BUTTON_SIZE: [f32; 2] = [24.0, 24.0];

/// Square, so up to eight fit across the window in one row.
const LAYER_BUTTON_SIZE: [f32; 2] = [28.0, 24.0];

/// Layer `n`'s mute key is `LAYER_KEYS[n]`, matching the number on its
/// button. As many as `MAX_LAYERS_RANGE` allows.
const LAYER_KEYS: [egui::Key; 8] = [
    egui::Key::Num1,
    egui::Key::Num2,
    egui::Key::Num3,
    egui::Key::Num4,
    egui::Key::Num5,
    egui::Key::Num6,
    egui::Key::Num7,
    egui::Key::Num8,
];

/// Everything the screen says about the current state, gathered once per
/// frame so that no two widgets can disagree about it.
fn readout(looper: &LooperState) -> indicator::Readout {
    let (loop_duration_secs, progress_fraction) = looper.loop_duration_and_progress();
    indicator::Readout {
        state: looper.state(),
        loop_duration_secs,
        progress_fraction,
        countdown_secs: looper.preroll_remaining_secs(),
        arming_fraction: looper.preroll_progress(),
        finishing_bar: looper.is_finishing_bar(),
    }
}

pub fn render(ui: &mut egui::Ui, looper: &mut LooperState) -> Option<Action> {
    let mut action = None;
    let (overdub_pressed, remove_pressed, redo_pressed, metronome_pressed) = ui.ctx().input(|i| {
        (
            // Edge-triggered, unlike the main control: none of these has
            // a long-press meaning, so there's nothing to time. The
            // spacebar is read where the looper is ticked, in `main.rs`.
            i.key_pressed(egui::Key::O),
            i.key_pressed(egui::Key::R),
            // Y, after Ctrl+Y: redo's key wherever there's an undo.
            i.key_pressed(egui::Key::Y),
            i.key_pressed(egui::Key::M),
        )
    });
    let readout = readout(looper);

    egui::Frame::default()
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            // Fixed top row, so the gear doesn't move when the indicator
            // grows below (e.g. the progress bar appearing).
            ui.horizontal(|ui| {
                ui.label("Looper Pedal");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("⚙").clicked() {
                        action = Some(Action::OpenSettings);
                    }
                    mode_switch(ui, looper);
                });
            });
            ui.add_space(8.0);

            // Directly under the title, above everything the controls
            // occupy: a release build has no console, so if this isn't
            // on screen the audio simply stops with no explanation.
            if let Some(fault) = looper.device_fault() {
                ui.colored_label(egui::Color32::RED, format!("Audio stopped: {fault}"));
                // Nothing here can reopen the device - the stream is
                // gone - so point at the one place that can.
                if ui.button("Choose a device").clicked() {
                    action = Some(Action::OpenSettings);
                }
                ui.add_space(8.0);
            }

            // Not an error - the loop is still playing - but it has
            // gaps in it, and nothing else on screen would say why.
            let underruns = looper.underruns();
            if underruns > 0 {
                ui.colored_label(
                    egui::Color32::from_rgb(200, 150, 60),
                    format!("Audio fell behind {underruns}x - raise Latency in Settings"),
                );
                ui.add_space(8.0);
            }

            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 10.0;

                // Button and spacebar feed one InputHandler (see
                // `LooperState::tick`), so they can't desync.
                let label =
                    indicator::press_button_label(&readout, looper.is_long_press_active());
                let button_response = ui.add_sized(
                    [90.0, 90.0],
                    egui::Button::new(egui::RichText::new(label).size(32.0)).corner_radius(45),
                );

                // Latched, not `is_pointer_button_down_on()` - see
                // `button_held` in `LooperState`. Read by the next
                // frame's tick, which is what the latch is for.
                if button_response.is_pointer_button_down_on() {
                    looper.set_button_held(true);
                }
                if !ui.input(|i| i.pointer.primary_down()) {
                    looper.set_button_held(false);
                }

                ui.add_space(4.0);
                indicator::state_indicator(ui, &readout);

                // Allocated at exactly the row's own width so the
                // centering layout above can center it - a plain
                // `horizontal` would take the full width and sit left.
                let row = egui::vec2(
                    CONTROL_BUTTON_SIZE[0] * 3.0 + ui.spacing().item_spacing.x * 2.0,
                    CONTROL_BUTTON_SIZE[1],
                );
                ui.allocate_ui_with_layout(
                    row,
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        // Each names its key, like the hint lines do.
                        // Kept short deliberately: the buttons are a
                        // fixed width so the row can be centered, and
                        // "Finish overdub (O)" doesn't fit in it.
                        let overdub_label = if looper.state() == LoopState::Overdubbing {
                            "Finish (O)"
                        } else {
                            "Overdub (O)"
                        };
                        let overdub_clicked = ui
                            .add_enabled_ui(looper.can_overdub(), |ui| {
                                ui.add_sized(CONTROL_BUTTON_SIZE, egui::Button::new(overdub_label))
                            })
                            .inner
                            .clicked();
                        if overdub_clicked || overdub_pressed {
                            looper.toggle_overdub();
                        }

                        let remove_clicked = ui
                            .add_enabled_ui(looper.can_remove_layer(), |ui| {
                                ui.add_sized(CONTROL_BUTTON_SIZE, egui::Button::new("Remove (R)"))
                            })
                            .inner
                            .clicked();
                        if remove_clicked || remove_pressed {
                            looper.remove_last_layer();
                        }

                        let redo_clicked = ui
                            .add_enabled_ui(looper.can_redo_layer(), |ui| {
                                ui.add_sized(CONTROL_BUTTON_SIZE, egui::Button::new("Redo (Y)"))
                            })
                            .inner
                            .clicked();
                        if redo_clicked || redo_pressed {
                            looper.redo_layer();
                        }
                    },
                );

                metronome_row(ui, looper, metronome_pressed);

                ui.colored_label(
                    egui::Color32::WHITE,
                    format!("Layers: {}/{}", looper.layer_count(), looper.max_layers()),
                );

                // Kept to three short lines: one long one would wrap
                // raggedly at this window width.
                ui.add_space(4.0);
                ui.label("Space or button: record / loop / stop");
                ui.label(format!(
                    "Hold either {:.1}s to clear",
                    looper.long_press_secs()
                ));
                ui.label("O: overdub  |  R: remove  |  Y: redo");
                ui.label("M: metronome  |  , and . : tempo");

                if looper.mode() == Mode::Extended {
                    extended_area(ui, looper);
                }
            });
        });

    action
}

/// The metronome toggle, lit while on and naming its tempo, with a step
/// slower and a step faster either side. Live - no trip to Settings.
fn metronome_row(ui: &mut egui::Ui, looper: &mut LooperState, toggle_pressed: bool) {
    // `,` and `.` sit under `<` and `>` on most layouts. Read as
    // presses, so holding one repeats.
    let (slower_pressed, faster_pressed) = ui
        .ctx()
        .input(|i| (i.key_pressed(egui::Key::Comma), i.key_pressed(egui::Key::Period)));

    // Measured and allocated like the control row, to center it.
    let spacing = ui.spacing().item_spacing.x;
    let row = egui::vec2(
        METRONOME_BUTTON_SIZE[0] + TEMPO_BUTTON_SIZE[0] * 2.0 + spacing * 2.0,
        METRONOME_BUTTON_SIZE[1],
    );
    ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        let slower = ui
            .add_sized(TEMPO_BUTTON_SIZE, egui::Button::new("−"))
            .on_hover_text("Slower (,)");
        if slower.clicked() || slower_pressed {
            looper.change_metronome_bpm(-1);
        }

        let toggle = egui::Button::selectable(
            looper.is_metronome_on(),
            format!("Metronome {} BPM (M)", looper.metronome_bpm()),
        );
        if ui.add_sized(METRONOME_BUTTON_SIZE, toggle).clicked() || toggle_pressed {
            looper.toggle_metronome();
        }

        let faster = ui
            .add_sized(TEMPO_BUTTON_SIZE, egui::Button::new("+"))
            .on_hover_text("Faster (.)");
        if faster.clicked() || faster_pressed {
            looper.change_metronome_bpm(1);
        }
    });
}

/// Both modes shown side by side, the current one selected, so the
/// control says where you are as well as where it goes.
fn mode_switch(ui: &mut egui::Ui, looper: &mut LooperState) {
    let mode = looper.mode();
    let clicked = ui
        .add_enabled_ui(looper.can_switch_mode(), |ui| {
            let option = |ui: &mut egui::Ui, this: Mode, label: &str| {
                ui.selectable_label(mode == this, label)
                    .on_disabled_hover_text("Stop the loop to switch")
                    .clicked()
                    && mode != this
            };
            // Right-to-left layout: added in reverse reading order.
            let extended = option(ui, Mode::Extended, "Extended");
            let mini = option(ui, Mode::Mini, "Mini");
            extended || mini
        })
        .inner;
    if clicked {
        looper.toggle_mode();
    }
}

/// What extended adds below the pedal. Drums and the mic will join the
/// layer row here.
fn extended_area(ui: &mut egui::Ui, looper: &mut LooperState) {
    ui.separator();
    layer_row(ui, looper);
}

/// One button per layer, numbered in recording order and lit while it
/// plays. A row rather than a list: eight layers fit across the window,
/// and wouldn't fit down it.
fn layer_row(ui: &mut egui::Ui, looper: &mut LooperState) {
    let count = looper.layer_count();
    if count == 0 {
        ui.weak("Layers appear here once there's a loop");
        return;
    }
    let key_pressed = ui
        .ctx()
        .input(|i| LAYER_KEYS.iter().position(|key| i.key_pressed(*key)));

    // Measured and allocated like the control row, to center it.
    let spacing = ui.spacing().item_spacing.x;
    let row = egui::vec2(
        count as f32 * (LAYER_BUTTON_SIZE[0] + spacing) - spacing,
        LAYER_BUTTON_SIZE[1],
    );
    ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        for index in 0..count {
            let playing = !looper.is_layer_muted(index);
            let button = egui::Button::selectable(playing, (index + 1).to_string());
            let clicked = ui
                .add_sized(LAYER_BUTTON_SIZE, button)
                .on_hover_text(if playing { "Mute" } else { "Unmute" })
                .clicked();
            if clicked || key_pressed == Some(index) {
                looper.toggle_layer_mute(index);
            }
        }
    });
    ui.label(format!("1-{count}: mute / unmute a layer"));
}
