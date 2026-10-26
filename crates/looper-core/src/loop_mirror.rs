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

/// The UI thread's own copy of the recorded loop, kept so it can be
/// saved: the layer stack itself lives inside the audio callback, where
/// nothing may reach in and read it.
///
/// Captured samples arrive through a ring buffer - the same lock-free
/// audio -> UI direction the telemetry uses - and takes are stored as
/// played, only laid out into layers when saving. So this costs about
/// what was actually recorded rather than a second full-size stack.
pub struct LoopMirror {
    captured: HeapCons<f32>,
    previous_state: LoopState,
    open_take: Option<Vec<f32>>,
    finished: Vec<Take>,
    /// Takes removed since the last one was recorded, newest last - what
    /// redo can bring back. Mirrors the layer stack's own count of them.
    removed: Vec<Take>,
    /// Taken from the audio thread's telemetry rather than from how many
    /// samples arrived here, a frame after a take ends - see `tick`.
    loop_len: usize,
    sample_rate: u32,
    save_pending: bool,
    /// Set if the capture ring overflowed while a take was open: the
    /// takes held here no longer match what is playing, so they must not
    /// be written over a good save. Cleared once the takes it applies to
    /// are gone - a fresh loop is trustworthy again, and latching it for
    /// the whole session would silently stop saving anything.
    lost_samples: bool,
}

impl LoopMirror {
    pub fn new(captured: HeapCons<f32>, sample_rate: u32, restored: Vec<Vec<f32>>) -> Self {
        let loop_len = restored.first().map(Vec::len).unwrap_or(0);
        Self {
            captured,
            previous_state: LoopState::Idle,
            open_take: None,
            finished: restored
                .into_iter()
                .map(|samples| Take { start: 0, samples })
                .collect(),
            removed: Vec::new(),
            loop_len,
            sample_rate,
            save_pending: false,
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
    ///
    /// `loop_len` and `take_start` come from the audio thread's
    /// telemetry: the length it settled on, and where the last overdub
    /// began.
    pub fn tick(&mut self, state: LoopState, loop_len: usize, take_start: usize) -> bool {
        // Drain first, so samples captured just before a transition
        // still land in the take that is ending. Anything arriving with
        // no take open belongs to one that has already finished, and is
        // dropped rather than corrupting the next.
        let mut scratch = [0.0f32; 4096];
        loop {
            let n = self.captured.pop_slice(&mut scratch);
            if n == 0 {
                break;
            }
            if let Some(take) = &mut self.open_take {
                take.extend_from_slice(&scratch[..n]);
            }
        }

        // The audio thread fixes the loop length a callback or two after
        // the state change that ended the take, so it's read on a later
        // frame than the transition.
        let mut out_of_date = false;
        if self.save_pending {
            self.loop_len = loop_len;
            if self.loop_len > 0 {
                self.save_pending = false;
                out_of_date = true;
            }
        }

        // Arming counts: a count-in's take opens on the audio thread, at
        // the bar line, before the state here says Recording.
        let is_capturing = |state| {
            matches!(
                state,
                LoopState::Arming | LoopState::Recording | LoopState::Overdubbing
            )
        };
        let capturing = is_capturing(state);
        let was_capturing = is_capturing(self.previous_state);

        if capturing && !was_capturing {
            self.open_take = Some(Vec::new());
            if state == LoopState::Overdubbing {
                // The new take is recorded into the slot a removed one
                // held.
                self.removed.clear();
            }
        }
        if state == LoopState::Recording && self.previous_state != LoopState::Recording {
            // The first take of a new loop replaces everything -
            // including any doubt about what the old one held, and what
            // redo could bring back. Not before: a count-in can still be
            // called off.
            self.finished.clear();
            self.removed.clear();
            self.loop_len = 0;
            self.lost_samples = false;
        }
        // Recording straight into an overdub is a take ending and another
        // beginning, though capturing never stops in between.
        let dubbed_on = self.previous_state == LoopState::Recording && state == LoopState::Overdubbing;
        if was_capturing && (!capturing || dubbed_on) {
            let take = self.open_take.take().unwrap_or_default();
            if self.previous_state == LoopState::Arming {
                // A pre-roll called off: whatever its count-in opened
                // never became a loop.
            } else if !take.is_empty() {
                // Overdubs begin wherever playback had reached; a first
                // take always begins at the top.
                let start = if self.previous_state == LoopState::Overdubbing {
                    take_start
                } else {
                    0
                };
                self.finished.push(Take {
                    start,
                    samples: take,
                });
                self.save_pending = true;
            } else if self.previous_state == LoopState::Recording {
                // Record pressed and undone before a sample arrived: the
                // old loop has been replaced by no loop. No length is
                // coming to wait for, so the copy on disk is stale as of
                // now - left alone it would come back at the next launch.
                out_of_date = true;
            }
        }
        if dubbed_on {
            self.open_take = Some(Vec::new());
        }

        self.previous_state = state;
        out_of_date
    }

    /// The take in progress is being thrown away: what it captured is
    /// dropped, and so is whatever of it is still on the way.
    pub fn abandon_take(&mut self) {
        self.open_take = None;
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

    pub fn clear(&mut self) {
        self.finished.clear();
        self.removed.clear();
        self.open_take = None;
        self.loop_len = 0;
        self.save_pending = false;
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
