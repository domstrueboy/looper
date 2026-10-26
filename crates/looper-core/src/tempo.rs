/// Where a guessed tempo is placed: one doubling wide, so exactly one
/// power-of-two beat count lands in it.
pub const GUESS_BPM: std::ops::Range<f64> = 70.0..140.0;

/// A loop's tempo, worked out from its length: how many beats it holds,
/// and the BPM that makes that many fit exactly.
///
/// Exact rather than rounded to a whole BPM, so the click lands on the
/// loop's own top every time round rather than drifting off it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopTempo {
    beats: u32,
    bpm: f64,
}

impl LoopTempo {
    /// For a loop played without the click. Loops are nearly always 1, 2,
    /// 4 or 8 bars, so the beat count is the power of two that puts the
    /// tempo in `GUESS_BPM`; halve or double put a wrong guess right.
    pub fn guess(loop_len: usize, sample_rate: u32) -> Self {
        let mut tempo = Self::with_beats(1, loop_len, sample_rate);
        while tempo.bpm < GUESS_BPM.start {
            tempo = tempo.doubled_unchecked();
        }
        tempo
    }

    /// For a loop played against the click, already a whole number of
    /// its bars: nothing to guess.
    pub fn from_bars(loop_len: usize, sample_rate: u32, bar_frames: f64, beats_per_bar: u32) -> Self {
        let bars = (loop_len as f64 / bar_frames).round().max(1.0) as u32;
        Self::with_beats(bars * beats_per_bar, loop_len, sample_rate)
    }

    fn with_beats(beats: u32, loop_len: usize, sample_rate: u32) -> Self {
        Self {
            beats,
            bpm: f64::from(beats) * 60.0 * f64::from(sample_rate) / loop_len.max(1) as f64,
        }
    }

    pub fn bpm(&self) -> f64 {
        self.bpm
    }

    pub fn beats(&self) -> u32 {
        self.beats
    }

    /// Half the tempo, half the beats - if the beats split evenly, so the
    /// click still lands on the loop's top, and the tempo stays in
    /// `range`.
    pub fn halved(&self, range: std::ops::RangeInclusive<f64>) -> Option<Self> {
        let halved = Self {
            beats: self.beats / 2,
            bpm: self.bpm / 2.0,
        };
        (self.beats.is_multiple_of(2) && range.contains(&halved.bpm)).then_some(halved)
    }

    /// Twice the tempo, twice the beats - if that stays in `range`.
    pub fn doubled(&self, range: std::ops::RangeInclusive<f64>) -> Option<Self> {
        let doubled = self.doubled_unchecked();
        range.contains(&doubled.bpm).then_some(doubled)
    }

    fn doubled_unchecked(&self) -> Self {
        Self {
            beats: self.beats * 2,
            bpm: self.bpm * 2.0,
        }
    }
}

#[cfg(test)]
#[path = "tempo_tests.rs"]
mod tests;
