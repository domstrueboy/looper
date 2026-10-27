use std::f32::consts::TAU;

/// High enough to cut through a guitar's fundamentals, low enough not to
/// be shrill.
pub const CLICK_FREQ_HZ: f32 = 1_000.0;

/// Beat one's click: a fifth up, so the bar is heard without counting.
pub const ACCENT_FREQ_HZ: f32 = 1_500.0;

/// Long enough to hear as a pitch, short enough to stay a tick.
pub const CLICK_MS: u32 = 20;

/// Half scale: a click over a loud loop still has room before the one
/// clip at the device.
pub const CLICK_PEAK: f32 = 0.5;

/// Rise time. Starting at full level would put a step at the front of the
/// burst, which is heard as a pop of its own.
const ATTACK_MS: u32 = 1;

/// The metronome's click, at `sample_rate`: a sine burst with a fast rise
/// and a fall to silence.
///
/// Built once, off the audio thread, at the rate the device actually
/// granted - so the callback only ever copies it, and a shared-mode
/// endpoint's own rate gets a click of the right pitch and length.
pub fn click(sample_rate: u32) -> Vec<f32> {
    burst(sample_rate, CLICK_FREQ_HZ)
}

/// Beat one's click: the same, higher.
pub fn accent(sample_rate: u32) -> Vec<f32> {
    burst(sample_rate, ACCENT_FREQ_HZ)
}

fn burst(sample_rate: u32, freq_hz: f32) -> Vec<f32> {
    let len = frames(CLICK_MS, sample_rate).max(2);
    let attack = frames(ATTACK_MS, sample_rate).clamp(1, len - 1);
    (0..len)
        .map(|i| {
            let phase = TAU * freq_hz * i as f32 / sample_rate as f32;
            CLICK_PEAK * envelope(i, attack, len) * phase.sin()
        })
        .collect()
}

fn frames(ms: u32, sample_rate: u32) -> usize {
    (u64::from(ms) * u64::from(sample_rate) / 1000) as usize
}

/// A straight rise over `attack`, then a squared fall reaching exactly
/// zero on the last sample. The squared fall is what makes it a tick
/// rather than a beep: most of the energy is at the front.
fn envelope(i: usize, attack: usize, len: usize) -> f32 {
    if i < attack {
        return i as f32 / attack as f32;
    }
    let remaining = (len - 1 - i) as f32 / (len - 1 - attack).max(1) as f32;
    remaining * remaining
}

#[cfg(test)]
#[path = "click_tests.rs"]
mod tests;
