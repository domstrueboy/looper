/// A bar as a grid of steps, and which of them sound.
///
/// The metronome is one step a beat with every step sounding; a drum
/// pattern is the same grid at a finer step, once per instrument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pattern {
    steps_per_beat: u32,
    steps_per_bar: u32,
    /// Bit `n` set = step `n` sounds. A bitmask so a pattern is `Copy`
    /// and can be handed to the audio thread without allocating.
    hits: u64,
}

impl Pattern {
    /// A click on every beat of a `beats`-beat bar. Which one is
    /// accented is the player's business: the step says.
    pub const fn metronome(beats: u32) -> Self {
        assert!(beats >= 1 && beats <= 8);
        Self {
            steps_per_beat: 1,
            steps_per_bar: beats,
            hits: (1 << beats) - 1,
        }
    }

    /// Up to 64 steps a bar - four bars of sixteenths, or one of 64ths.
    pub fn new(steps_per_beat: u32, steps_per_bar: u32, hits: u64) -> Self {
        assert!(steps_per_beat > 0 && (1..=64).contains(&steps_per_bar));
        Self {
            steps_per_beat,
            steps_per_bar,
            hits,
        }
    }

    fn sounds(&self, step: u32) -> bool {
        self.hits & (1 << step) != 0
    }
}

/// A step that sounds, somewhere in the block asked about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// Frames from the start of the block.
    pub offset: usize,
    /// Which step of the bar, from 0 - what accents and drum voices key
    /// on.
    pub step: u32,
}

/// Where a pattern's steps fall in time, at a tempo.
///
/// Driven by a sample position rather than by a clock of its own, so it
/// holds no state that could drift from the audio it lines up with, and a
/// test can ask about any position directly.
#[derive(Clone, Copy, Debug)]
pub struct Scheduler {
    pattern: Pattern,
    /// Fractional: 92 BPM at 44.1 kHz is 28,760.87 frames a beat, and
    /// rounding that once would let the grid wander a frame every few
    /// beats. Each step is rounded from its own exact position instead.
    frames_per_step: f64,
}

impl Scheduler {
    /// Fractional BPM, since a tempo worked out from a loop's length
    /// won't be a whole number.
    pub fn new(sample_rate: u32, bpm: f64, pattern: Pattern) -> Self {
        assert!(bpm > 0.0);
        let frames_per_beat = f64::from(sample_rate) * 60.0 / bpm;
        Self {
            pattern,
            frames_per_step: frames_per_beat / f64::from(pattern.steps_per_beat),
        }
    }

    /// The steps that sound in `frames` frames starting at position
    /// `start`, in order. Allocates nothing, so it runs in the callback;
    /// the work is bounded by how many steps fit in one block.
    pub fn events(&self, start: u64, frames: usize) -> Events {
        // At or just before `start`: rounding can't move a step by more
        // than half a frame, and a step is at least one frame long.
        let first = (start as f64 / self.frames_per_step).floor() as u64;
        Events {
            scheduler: *self,
            next: first,
            start,
            end: start + frames as u64,
        }
    }

    pub fn frames_per_step(&self) -> f64 {
        self.frames_per_step
    }

    pub fn frames_per_bar(&self) -> f64 {
        self.frames_per_step * f64::from(self.pattern.steps_per_bar)
    }

    fn position_of(&self, step: u64) -> u64 {
        (step as f64 * self.frames_per_step).round() as u64
    }
}

pub struct Events {
    scheduler: Scheduler,
    /// Counted from position zero, across bars.
    next: u64,
    start: u64,
    end: u64,
}

impl Iterator for Events {
    type Item = Event;

    fn next(&mut self) -> Option<Event> {
        loop {
            let at = self.scheduler.position_of(self.next);
            if at >= self.end {
                return None;
            }
            let step = (self.next % u64::from(self.scheduler.pattern.steps_per_bar)) as u32;
            self.next += 1;
            if at >= self.start && self.scheduler.pattern.sounds(step) {
                return Some(Event {
                    offset: (at - self.start) as usize,
                    step,
                });
            }
        }
    }
}

/// One one-shot sound playing out across blocks - a click, later a drum
/// hit. Holds only how far it has got, never the samples, so triggering
/// it in the callback allocates nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Voice {
    pos: Option<usize>,
}

impl Voice {
    /// Starts `sound` from the top, cutting off whatever was still
    /// playing - a click is over long before the next one.
    pub fn trigger(&mut self) {
        self.pos = Some(0);
    }

    /// Adds what's left of `sound` onto `out` at `gain`, as far as either
    /// runs. Added rather than written: it plays over the loop and the
    /// live signal, not instead of them.
    pub fn add_into(&mut self, sound: &[f32], out: &mut [f32], gain: f32) {
        let Some(pos) = self.pos else {
            return;
        };
        let remaining = sound.get(pos..).unwrap_or(&[]);
        let n = remaining.len().min(out.len());
        for (o, s) in out[..n].iter_mut().zip(remaining) {
            *o += s * gain;
        }
        self.pos = (pos + n < sound.len()).then_some(pos + n);
    }
}

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod tests;
