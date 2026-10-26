use super::*;

/// 60 BPM at 4 Hz: a beat every four frames, small enough to follow by
/// hand.
const RATE: u32 = 4;

fn offsets(scheduler: &Scheduler, start: u64, frames: usize) -> Vec<usize> {
    scheduler.events(start, frames).map(|e| e.offset).collect()
}

#[test]
fn the_metronome_sounds_on_every_beat() {
    let scheduler = Scheduler::new(RATE, 60.0, Pattern::METRONOME);
    let events: Vec<Event> = scheduler.events(0, 16).collect();
    assert_eq!(
        events,
        vec![
            Event { offset: 0, step: 0 },
            Event { offset: 4, step: 1 },
            Event { offset: 8, step: 2 },
            Event { offset: 12, step: 3 },
        ]
    );
}

#[test]
fn steps_count_on_into_the_next_bar() {
    let scheduler = Scheduler::new(RATE, 60.0, Pattern::METRONOME);
    let steps: Vec<u32> = scheduler.events(16, 8).map(|e| e.step).collect();
    assert_eq!(steps, vec![0, 1], "bar two starts over at step 0");
}

#[test]
fn offsets_are_from_the_start_of_the_block() {
    let scheduler = Scheduler::new(RATE, 60.0, Pattern::METRONOME);
    assert_eq!(offsets(&scheduler, 3, 4), vec![1], "the beat at 4");
}

#[test]
fn a_beat_on_the_block_edge_lands_in_exactly_one_block() {
    // Heard twice or not at all, otherwise.
    let scheduler = Scheduler::new(RATE, 60.0, Pattern::METRONOME);
    assert_eq!(offsets(&scheduler, 0, 4), vec![0]);
    assert_eq!(offsets(&scheduler, 4, 4), vec![0]);
}

#[test]
fn a_block_between_beats_has_none() {
    let scheduler = Scheduler::new(RATE, 60.0, Pattern::METRONOME);
    assert!(offsets(&scheduler, 1, 3).is_empty());
}

#[test]
fn a_fractional_tempo_does_not_drift() {
    // 92 BPM at 44.1 kHz is 28,760.87 frames a beat. Rounded once, the
    // grid would be 13 frames off after a hundred beats, and growing.
    let scheduler = Scheduler::new(44_100, 92.0, Pattern::METRONOME);
    let exact = 100.0 * 44_100.0 * 60.0 / 92.0;
    let start = exact as u64 - 10;
    let found = offsets(&scheduler, start, 20);
    assert_eq!(found.len(), 1);
    let at = start + found[0] as u64;
    assert!((at as f64 - exact).abs() <= 0.5, "beat 100 at {at}, exactly {exact}");
}

#[test]
fn small_blocks_see_every_beat_that_one_big_block_does() {
    // The callback hands over whatever block size the driver uses.
    let scheduler = Scheduler::new(44_100, 137.0, Pattern::METRONOME);
    let whole: Vec<u64> = scheduler.events(0, 200_000).map(|e| e.offset as u64).collect();
    let mut pieces = Vec::new();
    for block in 0..(200_000 / 64) {
        let start = block * 64;
        pieces.extend(scheduler.events(start, 64).map(|e| start + e.offset as u64));
    }
    assert_eq!(pieces, whole);
}

#[test]
fn only_the_steps_a_pattern_sets_sound() {
    // Eighths, sounding on the off-beats only.
    let pattern = Pattern::new(2, 8, 0b1010_1010);
    let scheduler = Scheduler::new(RATE, 60.0, pattern);
    let events: Vec<Event> = scheduler.events(0, 16).collect();
    let steps: Vec<u32> = events.iter().map(|e| e.step).collect();
    let at: Vec<usize> = events.iter().map(|e| e.offset).collect();
    assert_eq!(steps, vec![1, 3, 5, 7]);
    assert_eq!(at, vec![2, 6, 10, 14]);
}

#[test]
fn a_voice_plays_its_sound_across_blocks_and_stops() {
    let sound = [1.0, 2.0, 3.0];
    let mut voice = Voice::default();
    voice.trigger();

    let mut first = [10.0, 10.0];
    voice.add_into(&sound, &mut first);
    assert_eq!(first, [11.0, 12.0], "added, not written");

    let mut second = [0.0; 3];
    voice.add_into(&sound, &mut second);
    assert_eq!(second, [3.0, 0.0, 0.0]);
    assert!(!voice.is_playing());
}

#[test]
fn an_idle_voice_adds_nothing() {
    let mut voice = Voice::default();
    let mut out = [5.0; 2];
    voice.add_into(&[1.0], &mut out);
    assert_eq!(out, [5.0; 2]);
}

#[test]
fn retriggering_starts_the_sound_over() {
    let sound = [1.0, 2.0, 3.0];
    let mut voice = Voice::default();
    voice.trigger();
    voice.add_into(&sound, &mut [0.0; 2]);

    voice.trigger();
    let mut out = [0.0; 1];
    voice.add_into(&sound, &mut out);
    assert_eq!(out, [1.0]);
}
