/// One recorded layer, indexed by absolute position in the loop.
///
/// An overdub starts wherever playback happens to be and can be stopped
/// early, so a layer covers only the window it was recorded over:
/// `written` samples from `start` onward. Outside that window the buffer
/// still holds an earlier take's samples and is simply never read - which
/// is what lets a layer be reused without memsetting megabytes inside the
/// audio callback.
struct Layer {
    samples: Vec<f32>,
    start: usize,
    written: usize,
}

impl Layer {
    fn new(capacity: usize) -> Self {
        Self {
            samples: vec![0.0; capacity],
            start: 0,
            written: 0,
        }
    }

    /// How far `pos` is past the point this layer started recording,
    /// wrapping with the loop.
    fn offset_from_start(&self, pos: usize, len: usize) -> usize {
        (pos + len - self.start) % len
    }

    /// What this layer contributes at loop position `pos` - zero outside
    /// the window it was recorded over.
    fn sample_at(&self, pos: usize, len: usize) -> f32 {
        if self.offset_from_start(pos, len) < self.written {
            self.samples[pos]
        } else {
            0.0
        }
    }
}

/// A stack of aligned loop layers: the first recording fixes the loop
/// length, and each overdub adds another layer on top of it, playing back
/// as their sum. Layers are recorded and dropped independently, which is
/// the same data model multitrack needs later.
///
/// Nothing allocates outside `new`, so it's safe in a real-time callback -
/// which is also why every layer is a full `capacity`-sized buffer taken
/// up front, making the layer count a straight memory multiplier.
pub struct LoopStack {
    layers: Vec<Layer>,
    /// How many layers hold a finished take.
    count: usize,
    /// Loop length in samples, fixed when the first layer stops recording;
    /// 0 while empty. Playback wraps here - it does NOT grow while the
    /// first take is being recorded, which is what `recorded_len` is
    /// for.
    loop_len: usize,
    play_pos: usize,
    /// The layer being written to right now, if any. It sits just above
    /// `count` and is only counted once the take is finished.
    recording: Option<usize>,
    /// One bit per layer, set = left out of the mix. Only playback reads
    /// it: a muted layer is still there, recorded and saved, just silent.
    muted: u32,
}

impl LoopStack {
    /// `max_layers` buffers of `capacity` samples each, allocated now and
    /// never again.
    pub fn new(capacity: usize, max_layers: usize) -> Self {
        Self {
            layers: (0..max_layers.max(1))
                .map(|_| Layer::new(capacity))
                .collect(),
            count: 0,
            loop_len: 0,
            play_pos: 0,
            recording: None,
            muted: 0,
        }
    }

    /// Which layers to leave out, as a bitmask - bit `n` is layer `n`.
    /// Set by the callback each pass from what the UI thread published.
    pub fn set_muted(&mut self, muted: u32) {
        self.muted = muted;
    }

    fn capacity(&self) -> usize {
        self.layers[0].samples.len()
    }

    /// How much audio is recorded: the loop's fixed length, or how far
    /// the first take has got while it's still running - the elapsed
    /// time shown during recording comes from this.
    pub fn recorded_len(&self) -> usize {
        match self.recording {
            // Only the first take grows the loop; an overdub is bounded
            // by the length already fixed.
            Some(index) if self.loop_len == 0 => self.layers[index].written,
            _ => self.loop_len,
        }
    }

    fn is_empty(&self) -> bool {
        self.loop_len == 0
    }

    pub fn play_pos(&self) -> usize {
        self.play_pos
    }

    pub fn layer_count(&self) -> usize {
        self.count
    }

    fn is_full(&self) -> bool {
        self.count >= self.layers.len()
    }

    /// Starts the first layer, dropping whatever was there. The loop
    /// length isn't known until `finish_first_layer`, so this take simply
    /// appends from zero.
    pub fn begin_first_layer(&mut self) {
        self.clear();
        self.layers[0].start = 0;
        self.layers[0].written = 0;
        self.recording = Some(0);
    }

    /// Appends to the first layer, stopping at capacity - a take that
    /// reaches `max_loop_secs` stops growing rather than wrapping. How
    /// much has landed is `recorded_len`.
    pub fn record_first_layer(&mut self, input: &[f32]) {
        let Some(index) = self.recording else {
            return;
        };
        let capacity = self.capacity();
        let layer = &mut self.layers[index];
        let n = input.len().min(capacity - layer.written);
        layer.samples[layer.written..layer.written + n].copy_from_slice(&input[..n]);
        layer.written += n;
    }

    /// Fixes the loop length at whatever was recorded and rewinds to the
    /// top. A take with nothing in it leaves the stack empty.
    pub fn finish_first_layer(&mut self) {
        let Some(index) = self.recording.take() else {
            return;
        };
        self.loop_len = self.layers[index].written;
        self.play_pos = 0;
        self.count = usize::from(self.loop_len > 0);
    }

    /// Starts a layer aligned to the current playback position. False if
    /// there's no loop to overdub onto, or no layer left.
    pub fn begin_overdub(&mut self) -> bool {
        if self.is_empty() || self.is_full() {
            return false;
        }
        let index = self.count;
        self.layers[index].start = self.play_pos;
        self.layers[index].written = 0;
        self.recording = Some(index);
        true
    }

    /// Keeps the take just recorded. One with nothing in it is dropped
    /// rather than adding a silent layer.
    pub fn finish_overdub(&mut self) {
        let Some(index) = self.recording.take() else {
            return;
        };
        if self.layers[index].written > 0 {
            self.count = index + 1;
        }
    }

    /// Adds a layer already laid out over the whole loop, as a saved one
    /// is. The first fixes the loop length; the rest have to match it.
    /// For rebuilding a stack off the audio thread - false if it doesn't
    /// fit.
    pub fn add_layer(&mut self, samples: &[f32]) -> bool {
        if self.is_full() || samples.is_empty() || samples.len() > self.capacity() {
            return false;
        }
        if self.loop_len == 0 {
            self.loop_len = samples.len();
        } else if samples.len() != self.loop_len {
            return false;
        }

        let len = self.loop_len;
        let layer = &mut self.layers[self.count];
        layer.samples[..len].copy_from_slice(&samples[..len]);
        layer.start = 0;
        layer.written = len;
        self.count += 1;
        self.play_pos = 0;
        true
    }

    /// Drops the newest finished layer, leaving the loop length and
    /// playback position alone so the layers underneath keep playing.
    pub fn remove_last_layer(&mut self) {
        self.count = self.count.saturating_sub(1);
        if self.count == 0 {
            self.clear();
        }
    }

    /// Drops every layer and resets playback. Does not reallocate - the
    /// buffers keep their stale contents, which the recorded-window logic
    /// makes unreadable.
    pub fn clear(&mut self) {
        self.count = 0;
        self.loop_len = 0;
        self.play_pos = 0;
        self.recording = None;
    }

    /// One playback pass: mixes every layer into `out` at `gain_pct`
    /// percent of unity, advancing the shared playback position. Silence
    /// while empty.
    ///
    /// Layers all play at the level they were recorded at - there's no
    /// per-layer gain - so this is the only volume control over them.
    pub fn read_mixed(&mut self, out: &mut [f32], gain_pct: u32) {
        if self.loop_len == 0 {
            out.fill(0.0);
            return;
        }
        let gain = gain_factor(gain_pct);
        for sample in out.iter_mut() {
            *sample = self.mix_at(self.play_pos) * gain;
            self.play_pos = (self.play_pos + 1) % self.loop_len;
        }
    }

    /// Playback and overdub in one pass. Each sample is mixed into `out`
    /// *before* the matching `input` sample is recorded over it, so the
    /// take being played isn't echoed straight back on top of the
    /// player's own live signal - it comes back from the next pass on.
    /// Recording a second pass over the same layer sums into it rather
    /// than replacing it, so nothing already played is erased.
    pub fn read_mixed_with_overdub(&mut self, out: &mut [f32], input: &[f32], gain_pct: u32) {
        let len = self.loop_len;
        let Some(index) = self.recording.filter(|_| len > 0) else {
            self.read_mixed(out, gain_pct);
            return;
        };

        let gain = gain_factor(gain_pct);
        for (i, sample) in out.iter_mut().enumerate() {
            let pos = self.play_pos;
            *sample = self.mix_at(pos) * gain;

            let recorded = input.get(i).copied().unwrap_or(0.0);
            let layer = &mut self.layers[index];
            let offset = layer.offset_from_start(pos, len);
            if offset < layer.written {
                layer.samples[pos] += recorded;
            } else {
                layer.samples[pos] = recorded;
                // Walked forward one more sample from `start`; the window
                // is what makes the rest of the buffer unreadable.
                layer.written = offset + 1;
            }

            self.play_pos = (pos + 1) % len;
        }
    }

    /// Every layer's contribution at `pos`, summed. The take in progress
    /// is included - only over the part of it already recorded - so a
    /// second pass hears the first one come back around.
    ///
    /// Deliberately not clamped: four stacked takes can sum past full
    /// scale, and clamping that away here would make the loudness
    /// unrecoverable. The volume is applied to the whole sum by the
    /// callers, so turning it down still rescues a hot stack, and
    /// whatever is left over full scale is clipped once at the very edge
    /// of the app by `sample::to_pcm32`.
    fn mix_at(&self, pos: usize) -> f32 {
        let active = match self.recording {
            Some(index) => index + 1,
            None => self.count,
        };
        self.layers[..active]
            .iter()
            .enumerate()
            .filter(|(index, _)| self.muted & (1 << index) == 0)
            .map(|(_, layer)| layer.sample_at(pos, self.loop_len))
            .sum()
    }
}

/// The loop volume as a multiplier (100 = unchanged).
///
/// Divided by 100 rather than multiplied by 0.01: `0.01f32` is a hair
/// under a hundredth, so unity would come out a step off and playback
/// would no longer be bit-for-bit what was recorded - which is what the
/// mirror's layout test compares.
fn gain_factor(gain_pct: u32) -> f32 {
    gain_pct as f32 / 100.0
}

#[cfg(test)]
#[path = "loop_stack_tests.rs"]
mod tests;
