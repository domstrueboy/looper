use looper_core::action::Action;
use looper_core::config::{CLICK_VOLUME_PCT_RANGE, Mode, RecordMode, VOLUME_PCT_RANGE};

use crate::indicator;
use looper_core::looper::LooperState;
use std::time::Instant;

/// The secondary buttons share a size, so the row can be measured and
/// centered without laying it out twice. Three of them fit the window.
const CONTROL_BUTTON_SIZE: [f32; 2] = [88.0, 24.0];

/// The metronome row: a toggle naming its tempo, between a slower and a
/// faster button, then tap tempo.
const METRONOME_BUTTON_SIZE: [f32; 2] = [150.0, 24.0];
const TEMPO_BUTTON_SIZE: [f32; 2] = [24.0, 24.0];
const TAP_BUTTON_SIZE: [f32; 2] = [52.0, 24.0];

/// Leaves room beside the track for the label and the value.
const LEVEL_SLIDER_WIDTH: f32 = 150.0;

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
        dubs_on: looper.dubs_on(),
        waiting_to_hear: looper.is_waiting_to_hear(),
        stopping: looper.pending_stop(),
    }
}

pub fn render(ui: &mut egui::Ui, looper: &mut LooperState) -> Option<Action> {
    let mut action = None;
    let (redo_pressed, metronome_pressed) = ui.ctx().input(|i| {
        (
            // Edge-triggered, unlike the two pedals: neither has a
            // long-press meaning, so there's nothing to time. Space and S
            // are read where the looper is ticked, in `main.rs`.
            i.key_pressed(egui::Key::R),
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
                let label = indicator::press_button_label(&readout, looper.is_long_press_active());
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
                        // Stop is the second pedal: held like the round
                        // button, latched the same way, not clicked.
                        let stop_label = if looper.is_stop_long_press_active() {
                            "🗑 Cleared"
                        } else if looper.pending_stop().is_some() {
                            "Stop now (S)"
                        } else {
                            "Stop (S)"
                        };
                        let stop_response = ui
                            .add_enabled_ui(looper.can_stop_or_clear(), |ui| {
                                ui.add_sized(CONTROL_BUTTON_SIZE, egui::Button::new(stop_label))
                            })
                            .inner;
                        if stop_response.is_pointer_button_down_on() {
                            looper.set_stop_button_held(true);
                        }
                        if !ui.input(|i| i.pointer.primary_down()) {
                            looper.set_stop_button_held(false);
                        }

                        let undo_clicked = ui
                            .add_enabled_ui(looper.can_remove_layer(), |ui| {
                                ui.add_sized(CONTROL_BUTTON_SIZE, egui::Button::new("Undo"))
                            })
                            .inner
                            .clicked();
                        if undo_clicked {
                            looper.remove_last_layer();
                        }

                        let redo_clicked = ui
                            .add_enabled_ui(looper.can_redo_layer(), |ui| {
                                ui.add_sized(CONTROL_BUTTON_SIZE, egui::Button::new("Redo (R)"))
                            })
                            .inner
                            .clicked();
                        if redo_clicked || redo_pressed {
                            looper.redo_layer();
                        }
                    },
                );

                metronome_row(ui, looper, metronome_pressed);
                levels(ui, looper);

                ui.colored_label(
                    egui::Color32::WHITE,
                    format!("Layers: {}/{}", looper.layer_count(), looper.max_layers()),
                );

                // Kept to short lines: one long one would wrap raggedly
                // at this window width.
                ui.add_space(4.0);
                // Says which mode the main control is in: it changes what
                // a press does while dubbing.
                ui.label(match looper.record_mode() {
                    RecordMode::RecPlay => "Space or button: record / play / dub",
                    RecordMode::RecDub => "Space or button: record / dub / next dub",
                    RecordMode::RecDubPerPass => "Space or button: record / dub / play",
                });
                ui.label("S or double-tap: stop  |  R: redo");
                ui.label(format!(
                    "Hold {:.1}s - Space: undo  |  S: clear",
                    looper.long_press_secs()
                ));
                ui.label("M: metronome  |  , . : tempo  |  T: tap");

                if looper.mode() == Mode::Extended {
                    extended_area(ui, looper);
                }
            });
        });

    action
}

/// The metronome toggle, lit while on and naming its tempo, with a step
/// slower and a step faster either side. Live - no trip to Settings.
///
/// While a loop sets the tempo, the steps become halve and double: the
/// tempo is fixed by the loop's length, and those are the only changes
/// that keep the click on the loop's top.
fn metronome_row(ui: &mut egui::Ui, looper: &mut LooperState, toggle_pressed: bool) {
    // `,` and `.` sit under `<` and `>` on most layouts. Read as
    // presses, so holding one repeats.
    let (slower_pressed, faster_pressed, tap_pressed) = ui.ctx().input(|i| {
        (
            i.key_pressed(egui::Key::Comma),
            i.key_pressed(egui::Key::Period),
            // Not a repeat: holding T isn't tapping.
            i.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::T,
                        pressed: true,
                        repeat: false,
                        ..
                    }
                )
            }),
        )
    });

    // Measured and allocated like the control row, to center it.
    let spacing = ui.spacing().item_spacing.x;
    let row = egui::vec2(
        METRONOME_BUTTON_SIZE[0] + TEMPO_BUTTON_SIZE[0] * 2.0 + TAP_BUTTON_SIZE[0] + spacing * 3.0,
        METRONOME_BUTTON_SIZE[1],
    );
    let from_loop = looper.tempo_is_the_loops();
    ui.allocate_ui_with_layout(
        row,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let (label, hint, enabled) = if from_loop {
                ("½", "Half the tempo (,)", looper.can_halve_tempo())
            } else {
                ("−", "Slower (,)", true)
            };
            let slower = ui
                .add_enabled(
                    enabled,
                    egui::Button::new(label).min_size(TEMPO_BUTTON_SIZE.into()),
                )
                .on_hover_text(hint);
            if slower.clicked() || (slower_pressed && enabled) {
                if from_loop {
                    looper.halve_tempo();
                } else {
                    looper.change_metronome_bpm(-1);
                }
            }

            // A loop's tempo is rarely whole, and shown to a tenth so it
            // reads as measured rather than chosen.
            let tempo = if from_loop {
                format!("{:.1}", looper.tempo())
            } else {
                format!("{}", looper.tempo())
            };
            let toggle = egui::Button::selectable(
                looper.is_metronome_on(),
                format!("Metronome {tempo} BPM (M)"),
            );
            let toggle = ui.add_sized(METRONOME_BUTTON_SIZE, toggle);
            let toggle = if from_loop {
                toggle.on_hover_text("Tempo taken from the loop")
            } else {
                toggle
            };
            if toggle.clicked() || toggle_pressed {
                looper.toggle_metronome();
            }

            let (label, hint, enabled) = if from_loop {
                ("2×", "Double the tempo (.)", looper.can_double_tempo())
            } else {
                ("+", "Faster (.)", true)
            };
            let faster = ui
                .add_enabled(
                    enabled,
                    egui::Button::new(label).min_size(TEMPO_BUTTON_SIZE.into()),
                )
                .on_hover_text(hint);
            if faster.clicked() || (faster_pressed && enabled) {
                if from_loop {
                    looper.double_tempo();
                } else {
                    looper.change_metronome_bpm(1);
                }
            }

            // On the way down, like the pedals: a tap's time is when it
            // landed, and a click's release comes a varying while later.
            let tap = ui
                .add_enabled(
                    !from_loop,
                    egui::Button::new("Tap (T)").min_size(TAP_BUTTON_SIZE.into()),
                )
                .on_hover_text("Tap the beat, two or more times");
            let tap_down =
                tap.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed());
            if tap_down || (tap_pressed && !from_loop) {
                looper.tap_tempo(Instant::now());
            }
        },
    );
}

/// Loop and click level, live: heard while dragging, saved when let go.
/// Saving every frame of a drag would rewrite the config sixty times a
/// second for nothing.
fn levels(ui: &mut egui::Ui, looper: &mut LooperState) {
    // Scoped, so the width stays with these two.
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = LEVEL_SLIDER_WIDTH;
        let mut loop_pct = looper.loop_volume_pct();
        let loop_slider = ui.add(
            egui::Slider::new(&mut loop_pct, VOLUME_PCT_RANGE)
                .text("Loop")
                .suffix("%"),
        );
        if loop_slider.changed() {
            looper.set_loop_volume_pct(loop_pct);
        }
        let mut click_pct = looper.click_volume_pct();
        let click_slider = ui.add(
            egui::Slider::new(&mut click_pct, CLICK_VOLUME_PCT_RANGE)
                .text("Click")
                .suffix("%"),
        );
        if click_slider.changed() {
            looper.set_click_volume_pct(click_pct);
        }
        // A click on the track or a typed value changes it without a drag.
        let settled = |slider: &egui::Response| {
            slider.drag_stopped() || (slider.changed() && !slider.dragged())
        };
        if settled(&loop_slider) || settled(&click_slider) {
            looper.save_levels();
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
    ui.allocate_ui_with_layout(
        row,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
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
        },
    );
    ui.label(format!("1-{count}: mute / unmute a layer"));
}
