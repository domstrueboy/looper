use super::*;

/// 120 BPM at 48 kHz: a beat every 24,000 frames, a click 960 long.
const RATE: u32 = 48_000;
const BEAT: usize = 24_000;
const BLOCK: usize = 512;

/// Where each click begins: the first sounding sample after silence. The
/// click's own first sample is zero, so that's one frame before.
fn onsets(out: &[f32]) -> Vec<usize> {
    let mut onsets = Vec::new();
    let mut silent_for = usize::MAX;
    for (i, &s) in out.iter().enumerate() {
        if s != 0.0 {
            if silent_for > 100 {
                onsets.push(i - 1);
            }
            silent_for = 0;
        } else {
            silent_for = silent_for.saturating_add(1);
        }
    }
    onsets
}

fn free_run(metronome: &mut Metronome, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0; frames];
    for block in out.chunks_mut(BLOCK) {
        metronome.add_free(block);
    }
    out
}

#[test]
fn it_clicks_on_every_beat_whatever_the_block_size() {
    let mut metronome = Metronome::new(RATE, 120);
    let out = free_run(&mut metronome, BEAT * 3);
    assert_eq!(onsets(&out), vec![0, BEAT, 2 * BEAT]);
}

#[test]
fn restarting_counts_from_beat_one() {
    let mut metronome = Metronome::new(RATE, 120);
    free_run(&mut metronome, BEAT / 2);

    metronome.restart();
    let out = free_run(&mut metronome, BEAT + 2_000);

    assert_eq!(onsets(&out), vec![0, BEAT], "not half a beat in");
}

#[test]
fn on_a_loop_the_top_is_beat_one() {
    // A loop that isn't a whole number of beats - the grid starts over
    // at its top rather than running on.
    let loop_len = BEAT + 10_000;
    let mut metronome = Metronome::new(RATE, 120);
    let mut out = vec![0.0; loop_len + 2_000];
    // Starting past the last beat's click, 4,000 frames before the top.
    let mut pos = loop_len - 4_000;
    for block in out.chunks_mut(BLOCK) {
        metronome.add_on_loop(block, pos, loop_len);
        pos = (pos + block.len()) % loop_len;
    }
    assert_eq!(onsets(&out), vec![4_000, 4_000 + BEAT]);
}

#[test]
fn clicks_are_added_over_what_is_already_playing() {
    let mut metronome = Metronome::new(RATE, 120);
    let mut out = vec![0.25; BLOCK];
    metronome.add_free(&mut out);
    assert_eq!(out[0], 0.25, "the click's first sample is silence");
    assert!(out[1..100].iter().any(|&s| s != 0.25));
}

#[test]
fn a_tempo_change_keeps_the_place_in_the_bar() {
    // Half way to the next beat at 120 is half way at 60 too - the next
    // click comes half a 60 BPM beat later, not wherever 12,000 frames
    // lands on the slower grid.
    let mut metronome = Metronome::new(RATE, 120);
    free_run(&mut metronome, BEAT + BEAT / 2);

    metronome.set_bpm(60);
    let out = free_run(&mut metronome, 2 * BEAT + 2_000);

    assert_eq!(onsets(&out), vec![BEAT]);
}
