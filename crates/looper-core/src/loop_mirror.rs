use std::path::{Path, PathBuf};

use ringbuf::HeapCons;
use serde::{Deserialize, Serialize};
use ringbuf::traits::Consumer;

use crate::state_machine::LoopState;
use crate::config::AppConfig;
use crate::wav;

/// More layers than any config allows, so a stale file from a bigger
/// setting still gets cleaned up.
const MAX_SAVED_LAYERS: usize = 16;

const TEMPO_FILE: &str = "tempo.toml";

/// One recorded take: where in the loop it began, and what was played.
/// Saved layers come back in this shape too, already laid out from
/// position zero.
struct Take {
    start: usize,
    samples: Vec<f32>,
}

/// What the audio thread reports as each take ends, in the order they
/// end, on a ring of their own beside the samples. `samples` is how many
/// of the samples passed on since the last end were this take's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeEnd {
    /// A first take, which set the loop `loop_len` long - 0 for one with
    /// nothing in it, which leaves no loop.
    First { samples: usize, loop_len: usize },
    /// An overdub, begun at loop position `start`.
    Overdub { samples: usize, start: usize },
    /// A take thrown away: a dub a double press took back, a count-in
    /// called off after it opened, a take ended by a clear. `first` says
    /// it was a first take, whose opening wiped what redo could bring
    /// back - on the audio thread, and so here.
    Dropped { samples: usize, first: bool },
    /// The loop was cleared.
    Cleared,
}

/// The two rings the audio thread feeds this copy through.
pub struct Capture {
    pub samples: HeapCons<f32>,
    pub ends: HeapCons<TakeEnd>,
}

/// The UI thread's own copy of the recorded loop, kept so it can be
/// saved: the layer stack itself lives inside the audio callback, where
/// nothing may reach in and read it.
///
/// Fed by the audio thread with exactly what the stack recorded, and told
/// by it where each take ended - not by this thread's own state changes,
/// which reach the stack a callback later, and sometimes not at all. A
/// take is stored as played and only laid out into a layer when saving,
/// so this costs about what was actually recorded rather than a second
/// full-size stack.
pub struct LoopMirror {
    capture: Capture,
    /// Samples arrived but not yet claimed by an ended take: the one
    /// still being recorded.
    pending: Vec<f32>,
    previous_state: LoopState,
    finished: Vec<Take>,
    /// Takes removed since the last one was recorded, newest last - what
    /// redo can bring back. Mirrors the layer stack's own count of them.
    removed: Vec<Take>,
    /// As the audio thread settled it, from the first take's end.
    loop_len: usize,
    sample_rate: u32,
    /// Set if the capture ring overflowed: the takes held here no longer
    /// match what is playing, so they must not be written over a good
    /// save. Cleared as a new loop starts - latching it for the whole
    /// session would silently stop saving anything.
    lost_samples: bool,
}

impl LoopMirror {
    pub fn new(capture: Capture, sample_rate: u32, restored: Vec<Vec<f32>>) -> Self {
        let loop_len = restored.first().map(Vec::len).unwrap_or(0);
        Self {
            capture,
            pending: Vec::new(),
            previous_state: LoopState::Idle,
            finished: restored
                .into_iter()
                .map(|samples| Take { start: 0, samples })
                .collect(),
            removed: Vec::new(),
            loop_len,
            sample_rate,
            lost_samples: false,
        }
    }

    /// Samples went missing on the way here, so what's mirrored is no
    /// longer what's playing.
    pub fn note_lost_samples(&mut self) {
        self.lost_samples = true;
    }

    /// Call once per frame, after the state machine has been advanced.
    /// Returns whether the loop on disk is now out of date.
    pub fn tick(&mut self, state: LoopState) -> bool {
        // Ends before samples: a take's samples are passed on before its
        // end, so every end read here has all of its samples in by the
        // time they're drained.
        let mut ends = Vec::new();
        while let Some(end) = self.capture.ends.try_pop() {
            ends.push(end);
        }
        let mut scratch = [0.0f32; 4096];
        loop {
            let n = self.capture.samples.pop_slice(&mut scratch);
            if n == 0 {
                break;
            }
            self.pending.extend_from_slice(&scratch[..n]);
        }

        // Kept in step with this thread's own state for redo's sake: the
        // stack forgets what redo could bring back as a take opens, and
        // redo is offered from here, before any end can say so.
        let opens_take = matches!(state, LoopState::Recording | LoopState::Overdubbing);
        if opens_take && state != self.previous_state {
            self.removed.clear();
            if state == LoopState::Recording {
                // A new loop: whatever went missing from the old one no
                // longer matters.
                self.lost_samples = false;
            }
        }
        self.previous_state = state;

        let mut out_of_date = false;
        for end in ends {
            out_of_date |= self.end_take(end);
        }
        out_of_date
    }

    /// Settles one ended take. Returns whether the loop on disk is out of
    /// date because of it.
    fn end_take(&mut self, end: TakeEnd) -> bool {
        match end {
            TakeEnd::First { samples, loop_len } => {
                let take = self.claim(samples);
                // A first take replaces everything.
                self.finished.clear();
                self.removed.clear();
                self.loop_len = loop_len;
                if loop_len > 0 {
                    self.finished.push(Take {
                        start: 0,
                        samples: take,
                    });
                }
                // Even with nothing in it: the old loop is gone, and left
                // on disk it would come back at the next launch.
                true
            }
            TakeEnd::Overdub { samples, start } => {
                let take = self.claim(samples);
                if take.is_empty() || self.loop_len == 0 {
                    return false;
                }
                self.finished.push(Take {
                    start,
                    samples: take,
                });
                true
            }
            TakeEnd::Dropped { samples, first } => {
                self.claim(samples);
                if first {
                    self.removed.clear();
                }
                false
            }
            TakeEnd::Cleared => {
                self.finished.clear();
                self.removed.clear();
                self.loop_len = 0;
                true
            }
        }
    }

    /// The next `samples` arrived, as one take's.
    fn claim(&mut self, samples: usize) -> Vec<f32> {
        let rest = self.pending.split_off(samples.min(self.pending.len()));
        std::mem::replace(&mut self.pending, rest)
    }

    pub fn remove_last_layer(&mut self) {
        if let Some(take) = self.finished.pop() {
            self.removed.push(take);
        }
    }

    pub fn can_redo(&self) -> bool {
        !self.removed.is_empty()
    }

    pub fn redo_layer(&mut self) {
        if let Some(take) = self.removed.pop() {
            self.finished.push(take);
        }
    }

    /// Straight away, for the screen and the files. The audio thread's
    /// own ends for it follow, and settle to the same: the takes in
    /// flight are reported dropped, then cleared.
    pub fn clear(&mut self) {
        self.finished.clear();
        self.removed.clear();
        self.loop_len = 0;
        self.lost_samples = false;
    }

    /// Writes every layer into `directory` as a WAV, replacing whatever
    /// was there.
    pub fn save(&self, directory: &Path) -> Result<(), String> {
        if self.lost_samples {
            return Err("captured samples were lost; leaving the saved loop alone".to_string());
        }

        std::fs::create_dir_all(directory)
            .map_err(|e| format!("creating {}: {e}", directory.display()))?;
        // Clear first: a shorter loop must not leave layers of a longer
        // one behind to be loaded back alongside it. The tempo stays -
        // it's saved on its own, when it's known.
        delete_layers(directory);

        for (index, samples) in self.layers().iter().enumerate() {
            wav::write(&layer_path(directory, index), samples, self.sample_rate)?;
        }
        Ok(())
    }

    /// Every take laid out as a layer, in order.
    fn layers(&self) -> Vec<Vec<f32>> {
        self.finished
            .iter()
            .enumerate()
            .map(|(index, take)| {
                if index == 0 {
                    self.first_layer(take)
                } else {
                    self.layer(take)
                }
            })
            .collect()
    }

    /// The first take, which set the loop length rather than wrapping
    /// around one: anything past the length was cut, not summed back
    /// over the top - a take rounded down to the bar is the case that
    /// has any. Short of the length, the rest is silence.
    fn first_layer(&self, take: &Take) -> Vec<f32> {
        let mut layer = vec![0.0f32; self.loop_len];
        let n = take.samples.len().min(self.loop_len);
        layer[..n].copy_from_slice(&take.samples[..n]);
        layer
    }

    /// A take laid out over the whole loop, which is how it's stored and
    /// how the audio thread holds it.
    ///
    /// The first pass over a position replaces what's there and later
    /// passes sum into it - the same rule the layer stack applies while
    /// overdubbing, so a loop plays back the way it sounded.
    fn layer(&self, take: &Take) -> Vec<f32> {
        let mut layer = vec![0.0f32; self.loop_len];
        if self.loop_len == 0 {
            return layer;
        }

        for (i, &sample) in take.samples.iter().enumerate() {
            let at = (take.start + i) % self.loop_len;
            layer[at] = if i < self.loop_len {
                sample
            } else {
                layer[at] + sample
            };
        }
        layer
    }
}

/// The layers of a saved loop, in order, or nothing if there isn't one or
/// it no longer fits `settings`.
///
/// Every rule about whether a saved loop still applies lives here, so the
/// layer stack and the mirror are seeded from the same list. Seeding them
/// from different ones leaves the UI counting layers that aren't playing.
///
/// A loop is dropped whole if it was recorded at another sample rate
/// (there's no resampling), if the files disagree on a length, or if it
/// is longer than `max_loop_secs` now allows. Layers past `max_layers`
/// are dropped individually instead - they're independent takes, so the
/// ones underneath are still exactly what was played.
pub fn load(directory: &Path, settings: &AppConfig) -> Vec<Vec<f32>> {
    let mut layers: Vec<Vec<f32>> = Vec::new();

    for index in 0..MAX_SAVED_LAYERS {
        let Some((samples, rate)) = wav::read(&layer_path(directory, index)) else {
            break;
        };
        if rate != settings.sample_rate || samples.is_empty() {
            return Vec::new();
        }
        if layers
            .first()
            .is_some_and(|first| first.len() != samples.len())
        {
            return Vec::new();
        }
        layers.push(samples);
    }

    let capacity = settings.max_loop_secs as usize * settings.sample_rate as usize;
    if layers.first().is_some_and(|first| first.len() > capacity) {
        return Vec::new();
    }
    layers.truncate(settings.max_layers as usize);
    layers
}

/// The whole saved loop, tempo included.
pub fn delete(directory: &Path) {
    delete_layers(directory);
    let _ = std::fs::remove_file(directory.join(TEMPO_FILE));
}

fn delete_layers(directory: &Path) {
    for index in 0..MAX_SAVED_LAYERS {
        let _ = std::fs::remove_file(layer_path(directory, index));
    }
}

/// How many beats the saved loop holds. Only the beats: the tempo follows
/// from them and the loop's length exactly, and saving it as well would
/// only let the two disagree.
#[derive(Serialize, Deserialize)]
struct SavedTempo {
    beats: u32,
}

pub fn save_beats(directory: &Path, beats: u32) -> Result<(), String> {
    std::fs::create_dir_all(directory)
        .map_err(|e| format!("creating {}: {e}", directory.display()))?;
    let text = toml::to_string(&SavedTempo { beats })
        .map_err(|e| format!("serializing the tempo: {e}"))?;
    let path = directory.join(TEMPO_FILE);
    std::fs::write(&path, text).map_err(|e| format!("writing {}: {e}", path.display()))
}

/// `None` for no file, or one that doesn't read - the tempo is guessed
/// again then, which is what a loop saved before this existed gets too.
pub fn load_beats(directory: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(directory.join(TEMPO_FILE)).ok()?;
    let saved: SavedTempo = toml::from_str(&text).ok()?;
    (saved.beats > 0).then_some(saved.beats)
}

/// Numbered from 1, so the filenames read the way the layer counter does.
fn layer_path(directory: &Path, index: usize) -> PathBuf {
    directory.join(format!("layer-{}.wav", index + 1))
}

#[cfg(test)]
#[path = "loop_mirror_tests.rs"]
mod tests;
