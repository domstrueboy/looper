// Mono 32-bit PCM WAV, by hand: a header is 44 bytes of little-endian
// fields, which is cheaper than a dependency and leaves the saved loop as
// something any audio editor can open - which is the format the eventual
// "export loop" feature wants anyway.

use std::path::Path;

use crate::sample;

const HEADER_BYTES: u32 = 36;
const FMT_CHUNK_BYTES: u32 = 16;
const PCM: u16 = 1;
const MONO: u16 = 1;
const BITS: u16 = 32;
const BYTES_PER_SAMPLE: u32 = BITS as u32 / 8;

pub fn write(path: &Path, samples: &[f32], sample_rate: u32) -> Result<(), String> {
    std::fs::write(path, encode(samples, sample_rate))
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

/// The samples and the rate they were recorded at, or `None` if this
/// isn't a mono 32-bit PCM WAV.
///
/// The file stays 32-bit integer while the core works in f32, so loops
/// saved by earlier builds still load and any audio editor can still
/// open one - the conversion happens here, at the edge.
pub fn read(path: &Path) -> Option<(Vec<f32>, u32)> {
    decode(&std::fs::read(path).ok()?)
}

fn encode(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_bytes = samples.len() as u32 * BYTES_PER_SAMPLE;
    let mut wav = Vec::with_capacity((HEADER_BYTES + 8 + data_bytes) as usize);

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(HEADER_BYTES + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVE");

    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&FMT_CHUNK_BYTES.to_le_bytes());
    wav.extend_from_slice(&PCM.to_le_bytes());
    wav.extend_from_slice(&MONO.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * BYTES_PER_SAMPLE).to_le_bytes());
    wav.extend_from_slice(&(BYTES_PER_SAMPLE as u16).to_le_bytes());
    wav.extend_from_slice(&BITS.to_le_bytes());

    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for &sample in samples {
        wav.extend_from_slice(&sample::to_pcm32(sample).to_le_bytes());
    }
    wav
}

fn decode(bytes: &[u8]) -> Option<(Vec<f32>, u32)> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }

    let mut sample_rate = None;
    let mut samples = None;

    // Chunks can arrive in any order, and files often carry ones we
    // don't care about, so walk rather than assume the layout we write.
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let body = bytes.get(at + 8..at + 8 + size)?;

        match id {
            b"fmt " => {
                if body.len() < FMT_CHUNK_BYTES as usize {
                    return None;
                }
                let format = u16::from_le_bytes(body[0..2].try_into().ok()?);
                let channels = u16::from_le_bytes(body[2..4].try_into().ok()?);
                let bits = u16::from_le_bytes(body[14..16].try_into().ok()?);
                if (format, channels, bits) != (PCM, MONO, BITS) {
                    return None;
                }
                sample_rate = Some(u32::from_le_bytes(body[4..8].try_into().ok()?));
            }
            b"data" => {
                samples = Some(
                    body.as_chunks::<{ BYTES_PER_SAMPLE as usize }>()
                        .0
                        .iter()
                        .map(|&s| sample::from_pcm32(i32::from_le_bytes(s)))
                        .collect(),
                );
            }
            _ => {}
        }

        // Chunk bodies are padded to an even length.
        at += 8 + size + (size & 1);
    }

    Some((samples?, sample_rate?))
}

#[cfg(test)]
#[path = "wav_tests.rs"]
mod tests;
