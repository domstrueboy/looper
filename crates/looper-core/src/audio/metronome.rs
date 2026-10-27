use super::click::{accent, click};
use super::scheduler::{Pattern, Scheduler, Voice};

/// The click track: the click sound, where its beats fall, and the voice
/// playing it. Lives in the output callback; everything it holds is
/// built in `new`, so playing it allocates nothing.
pub struct Metronome {
    click: Vec<f32>,
    accent: Vec<f32>,
    pattern: Pattern,
    scheduler: Scheduler,
    voice: Voice,
    /// Whether the voice is playing beat one's click rather than the
    /// others'.
    accented: bool,
    /// Frames counted since `restart`, for when there's no loop to keep
    /// time with.
    free_pos: u64,
    sample_rate: u32,
    bpm: f64,
    /// The click's level, 1.0 its own: set live from the looper screen.
    gain: f32,
}

impl Metronome {
    pub fn new(sample_rate: u32, bpm: u32, beats_per_bar: u32) -> Self {
        let pattern = Pattern::metronome(beats_per_bar);
        Self {
            click: click(sample_rate),
            accent: accent(sample_rate),
            pattern,
            scheduler: Scheduler::new(sample_rate, f64::from(bpm), pattern),
            voice: Voice::default(),
            accented: false,
            free_pos: 0,
            sample_rate,
            bpm: f64::from(bpm),
            gain: 1.0,
        }
    }

    /// Changes tempo without losing the place in the bar: half way to the
    /// next beat stays half way. Left as a frame count, the old position
    /// would land anywhere on the new grid - two clicks in a row, or a
    /// long gap. Cheap enough to call every callback.
    pub fn set_bpm(&mut self, bpm: f64) {
        if bpm == self.bpm {
            return;
        }
        let old_step = self.scheduler.frames_per_step();
        self.scheduler = Scheduler::new(self.sample_rate, bpm, self.pattern);
        self.free_pos = (self.free_pos as f64 * self.scheduler.frames_per_step() / old_step) as u64;
        self.bpm = bpm;
    }

    /// How long a bar is at the current tempo - what a loop recorded
    /// against the click is rounded to.
    pub fn bar_frames(&self) -> f64 {
        self.scheduler.frames_per_bar()
    }

    /// Frames from where its own count has got to, to the end of the
    /// first bar - where a count-in's take opens. `None` once past it.
    pub fn frames_to_bar_line(&self) -> Option<usize> {
        let line = self.bar_frames().round() as u64;
        line.checked_sub(self.free_pos)
            .map(|frames| frames as usize)
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
    }

    /// Counts again from beat one, so the pre-roll's count-in and the
    /// take after it each open on a click.
    pub fn restart(&mut self) {
        self.free_pos = 0;
    }

    /// Clicks on its own count, for when there's no loop playing.
    pub fn add_free(&mut self, out: &mut [f32]) {
        self.add_from(out, self.free_pos);
        self.free_pos += out.len() as u64;
    }

    /// Clicks in time with a playing loop, `out` starting at loop
    /// position `pos`. Beat one is the top of the loop, so the count
    /// starts over each time round.
    pub fn add_on_loop(&mut self, out: &mut [f32], pos: usize, loop_len: usize) {
        let to_top = loop_len.saturating_sub(pos).min(out.len());
        let (before, after) = out.split_at_mut(to_top);
        self.add_from(before, pos as u64);
        // A block crosses the top at most once, unless the loop is
        // shorter than a block - a few milliseconds, not worth a click.
        self.add_from(after, 0);
    }

    /// Plays on through `out`, retriggering at every beat that falls in
    /// it - a beat cuts off whatever is left of the last click.
    fn add_from(&mut self, out: &mut [f32], start: u64) {
        let mut done = 0;
        for beat in self.scheduler.events(start, out.len()) {
            let sound = if self.accented {
                &self.accent
            } else {
                &self.click
            };
            self.voice
                .add_into(sound, &mut out[done..beat.offset], self.gain);
            self.voice.trigger();
            self.accented = beat.step == 0;
            done = beat.offset;
        }
        let sound = if self.accented {
            &self.accent
        } else {
            &self.click
        };
        self.voice.add_into(sound, &mut out[done..], self.gain);
    }
}

#[cfg(test)]
#[path = "metronome_tests.rs"]
mod tests;
