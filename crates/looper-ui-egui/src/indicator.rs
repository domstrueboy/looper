use looper_core::state_machine::LoopState;

/// Everything the looper screen displays about the current state,
/// gathered into one value rather than passed as a row of same-typed
/// arguments.
pub struct Readout {
    pub state: LoopState,
    /// Recorded loop length in seconds; 0.0 when empty.
    pub loop_duration_secs: f32,
    /// Playback position through the loop, 0.0-1.0, meaningful once
    /// something is recorded.
    pub progress_fraction: f32,
    /// Seconds left of the pre-roll, meaningful while arming.
    pub countdown_secs: f32,
    /// How far through the pre-roll, 0.0-1.0, likewise.
    pub arming_fraction: f32,
    /// Stop was pressed, and recording carries on to the bar line.
    pub finishing_bar: bool,
    /// A press while dubbing keeps the layer and dubs the next, rather
    /// than plays.
    pub dubs_on: bool,
}

/// Label for the main button: what a short press does next, or a trash
/// icon while a long-press-clear is being held. Arming gets a cancel
/// cross of its own, because a press there throws the pending recording
/// away rather than stopping one that's under way. A press never stops,
/// so nothing here shows pause or stop: playing shows record, since a
/// press dubs.
///
/// Every glyph here has to exist in egui's bundled fonts, which carry a
/// limited subset - anything else renders as an empty box.
pub fn press_button_label(readout: &Readout, long_press_active: bool) -> &'static str {
    if long_press_active {
        return "🗑";
    }
    match readout.state {
        LoopState::Idle | LoopState::Looping => "⏺",
        LoopState::Overdubbing if readout.dubs_on => "⏺",
        LoopState::Arming => "✖",
        LoopState::Recording => "⏹",
        LoopState::Overdubbing | LoopState::Stopped => "▶",
    }
}

/// Colored state circle, label, and loop duration / progress.
pub fn state_indicator(ui: &mut egui::Ui, readout: &Readout) {
    let (color, label) = match readout.state {
        LoopState::Idle => (egui::Color32::GRAY, "Empty"),
        LoopState::Arming => (egui::Color32::from_rgb(60, 140, 255), "Get ready"),
        // Says why a press that should have stopped it hasn't, yet.
        LoopState::Recording if readout.finishing_bar => {
            (egui::Color32::RED, "Recording to the bar line")
        }
        LoopState::Recording => (egui::Color32::RED, "Recording"),
        LoopState::Looping => (egui::Color32::from_rgb(40, 200, 40), "Looping"),
        LoopState::Overdubbing => (egui::Color32::from_rgb(255, 110, 40), "Overdubbing"),
        LoopState::Stopped => (egui::Color32::from_rgb(230, 170, 30), "Stopped"),
    };

    ui.horizontal(|ui| {
        let (rect, _response) =
            ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());

        let radius = if matches!(
            readout.state,
            LoopState::Recording | LoopState::Overdubbing | LoopState::Arming
        ) {
            // Gentle pulse while something is being captured, or about to
            // be.
            let t = ui.input(|i| i.time) as f32;
            5.5 + 2.0 * (t * 3.0).sin().abs()
        } else {
            7.0
        };
        ui.painter().circle_filled(rect.center(), radius, color);

        ui.label(label);
    });

    match readout.state {
        LoopState::Idle => {}
        LoopState::Arming => {
            // Laid out exactly like the states below: the time as a
            // label where the recording and loop times appear, the bar
            // beneath it. Not the bar's own `text`, which egui draws on
            // the left, where a shrinking number behind a growing fill
            // reads as a contradiction.
            ui.label(format!("{:.1}s", readout.countdown_secs));
            // Gray rather than the playing bar's colour, so a glance
            // can't confuse waiting with playing. Recording starts when
            // it's full.
            ui.add(
                egui::ProgressBar::new(readout.arming_fraction).fill(egui::Color32::from_gray(200)),
            );
        }
        LoopState::Recording => {
            ui.label(format!("{:.1}s", readout.loop_duration_secs));
        }
        LoopState::Looping | LoopState::Stopped | LoopState::Overdubbing => {
            ui.label(format!("{:.1}s loop", readout.loop_duration_secs));
            // Frozen at the last position while Stopped rather than
            // vanishing - it shows where playback will resume.
            ui.add(egui::ProgressBar::new(readout.progress_fraction));
        }
    }
}
