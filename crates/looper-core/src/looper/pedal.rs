//! What the two controls do - press, double press and hold on the main
//! one, press and hold on stop - from every state, in every record mode.
//! `every_gesture_from_every_state_in_every_mode` in the looper's tests
//! is the whole of it as one table.

use std::time::Instant;

use super::{AudioStart, LooperState, TempoSource};
use crate::config::{RecordMode, RecordStart, StopMode};
use crate::input::InputEvent;
use crate::state_machine::LoopState;

impl LooperState {
    /// The stop control - the second pedal, and what a double press does
    /// on the first. A take or a dub ends and is kept, a count-in is
    /// called off. In REC→DUB, where the main control never leaves
    /// dubbing, it steps down: dubbing to playing, playing to stopped.
    /// A playing loop stops the `StopMode` way; stopping it again while
    /// that runs means now.
    pub fn stop(&mut self) {
        match self.state() {
            LoopState::Idle | LoopState::Stopped => return,
            // Now, not at the bar line: the length is still rounded,
            // there's just nothing left to play out.
            LoopState::Recording => {
                self.stop_at = None;
                self.end_take(false);
                self.state_machine.press();
            }
            LoopState::Arming => {
                self.state_machine.press();
                self.preroll.cancel();
                self.audio_start = None;
            }
            LoopState::Overdubbing if self.config.record_mode == RecordMode::RecDub => {
                self.state_machine.toggle_overdub();
            }
            LoopState::Looping | LoopState::Overdubbing
                if self.config.stop_mode == StopMode::Immediate || self.stop_pending =>
            {
                self.stop_pending = false;
                self.state_machine.press();
            }
            // The dub is kept now; the loop plays on without it growing.
            LoopState::Looping | LoopState::Overdubbing => {
                if self.state() == LoopState::Overdubbing {
                    self.state_machine.toggle_overdub();
                }
                self.stop_pending = true;
                self.control.request_stop(self.config.stop_mode);
            }
        }
        self.control.publish_state(self.state_machine.state());
    }

    /// Calls off a stop under way - before whatever state comes next is
    /// published, so the audio thread can't play on past where it held.
    /// Stopped included: the request stays set there, harmlessly, until
    /// this.
    fn cancel_stop(&mut self) {
        self.stop_pending = false;
        self.control.cancel_stop();
    }

    /// Publishes the resulting state in the same step as changing it, so
    /// that pairing can't be forgotten at a call site.
    pub(super) fn apply(&mut self, event: InputEvent, now: Instant) {
        match event {
            InputEvent::Press => self.press(now),
            InputEvent::DoublePress => self.double_press(),
            InputEvent::LongPress => self.hold(),
            InputEvent::None => {}
        }
    }

    /// Never stops - that's S, or a double press. From empty it records,
    /// then walks on through the record mode's cycle; stopped, it plays.
    fn press(&mut self, now: Instant) {
        self.pressed_from = Some(self.state());
        // Space while a stop plays out changes your mind: the loop goes
        // on, and that's all - no dub opened by the same press.
        if self.stop_pending {
            self.cancel_stop();
            return;
        }
        // A stop that ran its course leaves its request set; cleared
        // before playing again, or the loop would park straight away.
        self.control.cancel_stop();
        // A second press while waiting for the bar line means now: the
        // take is still rounded, the rest plays silent.
        if self.state() == LoopState::Recording && self.stop_at.is_none() {
            self.stop_at = self.bar_line_ahead();
            if self.stop_at.is_some() {
                return;
            }
        }
        self.stop_at = None;
        match self.state() {
            // Only for the first recording - once a loop is playing
            // you're already in time with it.
            LoopState::Idle if self.config.record_start == RecordStart::CountIn => {
                self.state_machine.arm();
                self.audio_start = Some(AudioStart::CountIn(now));
            }
            LoopState::Idle if self.config.record_start == RecordStart::Detect => {
                self.state_machine.arm();
                self.audio_start = Some(AudioStart::Detect);
            }
            LoopState::Idle
                if self.config.record_start == RecordStart::Delay && self.preroll.is_enabled() =>
            {
                self.state_machine.arm();
                self.preroll.start(now);
            }
            LoopState::Recording => self.end_take(self.dubs_after_take()),
            // With every layer used there's nothing to dub into, and a
            // press does nothing.
            LoopState::Looping => {
                if self.can_overdub() {
                    self.state_machine.toggle_overdub();
                }
            }
            // No state change to publish: dubbing goes on.
            LoopState::Overdubbing if self.dubs_on() => {
                return self.control.request_next_layer();
            }
            LoopState::Overdubbing => self.state_machine.toggle_overdub(),
            // Records at once, calls a wait off, plays a stopped loop.
            LoopState::Idle | LoopState::Arming | LoopState::Stopped => {
                self.state_machine.press();
                self.preroll.cancel();
                self.audio_start = None;
            }
        }
        self.control.publish_state(self.state_machine.state());
    }

    /// What S does - but the first half already acted as it went down,
    /// so whatever that started is taken back first: a dub it opened is
    /// dropped rather than left as a sliver of a layer, and a take or a
    /// count-in it began on an empty looper leaves it empty.
    fn double_press(&mut self) {
        let from = self.pressed_from.take();
        match from {
            Some(LoopState::Idle) => return self.clear(),
            // The first half already called the count-in off.
            Some(LoopState::Arming) => return,
            _ => {}
        }
        // A dub going now can only be the first half's: one going
        // before it, it closed - or in REC→DUB split, opening this one.
        if self.state() == LoopState::Overdubbing {
            self.control.request_abandon_take();
            self.state_machine.toggle_overdub();
        }
        // REC→DUB's stop from dubbing is to playing, where the take-back
        // has already left it.
        if from == Some(LoopState::Overdubbing) && self.config.record_mode == RecordMode::RecDub {
            return self.control.publish_state(self.state_machine.state());
        }
        self.stop();
    }

    /// Undo, one layer a hold. The press it started with has already
    /// acted, so that's taken back first. Held while dubbing, the dub in
    /// progress is the layer undone; held while recording the first
    /// take, the take is - leaving the looper empty. A finished first
    /// take never is: that's a clear, on the stop control.
    fn hold(&mut self) {
        let from = self.pressed_from.take();
        match from {
            Some(LoopState::Idle | LoopState::Recording) => return self.clear(),
            // The press called a count-in off: nothing left to undo.
            Some(LoopState::Arming) => return,
            _ => {}
        }
        // Only the press can have opened a dub that's going now - from
        // playing, or in REC→DUB as the next layer, the one before it
        // being what's undone.
        if self.state() == LoopState::Overdubbing {
            self.control.request_abandon_take();
            self.state_machine.toggle_overdub();
        }
        // The press played it.
        if from == Some(LoopState::Stopped) {
            self.state_machine.press();
        }
        self.control.publish_state(self.state_machine.state());
        self.undo_pending = true;
    }

    /// Where a take stopped now would be rounded up to, if the metronome
    /// is on and that's still ahead - in frames recorded, the same count
    /// the audio thread rounds. Nearer the bar line behind, or past what
    /// a loop can hold, there's nothing to wait for.
    fn bar_line_ahead(&self) -> Option<usize> {
        if !self.metronome_on {
            return None;
        }
        let bar = self.bar_frames();
        let played = self.control.loop_len() as f64;
        let target = ((played / bar).round().max(1.0) * bar).round();
        let capacity = f64::from(self.config.max_loop_secs) * f64::from(self.sample_rate);
        (target > played && target <= capacity).then_some(target as usize)
    }

    /// Ends a first take, noting what its tempo will come from: the
    /// length isn't fixed until the audio thread has handled the stop.
    /// `into_dub` goes straight on into an overdub - one state change,
    /// which the audio thread takes as "close the take, fix the length,
    /// open a layer at the top". The caller publishes.
    pub(super) fn end_take(&mut self, into_dub: bool) {
        self.tempo_pending = Some(if self.metronome_on {
            TempoSource::Click(self.bar_frames())
        } else {
            TempoSource::Guess
        });
        self.loop_tempo = None;
        self.state_machine.press();
        if into_dub {
            self.state_machine.toggle_overdub();
        }
    }

    /// Whether ending the take goes straight on into a dub. Not with one
    /// layer allowed: there'd be nothing to dub into.
    pub(super) fn dubs_after_take(&self) -> bool {
        self.config.record_mode.dubs_after_take() && self.max_layers > 1
    }

    /// Whether a press now keeps the dub in progress and dubs the next
    /// layer, rather than plays: REC→DUB, with a layer left after the
    /// one being recorded.
    pub fn dubs_on(&self) -> bool {
        self.state() == LoopState::Overdubbing
            && self.config.record_mode == RecordMode::RecDub
            && self.layer_count() + 2 <= self.max_layers
    }

    pub(super) fn clear(&mut self) {
        self.cancel_stop();
        self.undo_pending = false;
        self.stop_at = None;
        self.audio_start = None;
        self.loop_tempo = None;
        self.tempo_pending = None;
        self.state_machine.clear();
        self.preroll.cancel();
        self.control.publish_state(self.state_machine.state());
        self.control.request_clear();
        self.forget_mutes_from(0);
        self.mirror.clear();
        self.writer.delete();
    }
}
