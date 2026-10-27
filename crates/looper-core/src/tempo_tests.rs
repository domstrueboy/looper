use super::*;

const RATE: u32 = 44_100;

fn secs(s: f64) -> usize {
    (s * f64::from(RATE)) as usize
}

const RANGE: std::ops::RangeInclusive<f64> = 40.0..=300.0;

#[test]
fn a_guess_lands_in_the_playable_range() {
    // 2.6 s: one beat is 23 BPM, four are 92.3.
    let tempo = LoopTempo::guess(secs(2.6), RATE, 4);
    assert_eq!(tempo.beats(), 4);
    assert!((tempo.bpm() - 92.307).abs() < 0.01, "got {}", tempo.bpm());
}

#[test]
fn a_long_loop_is_guessed_as_more_bars() {
    // 16 s is eight bars at 120, not four at 60.
    let tempo = LoopTempo::guess(secs(16.0), RATE, 4);
    assert_eq!(tempo.beats(), 32);
    assert!((tempo.bpm() - 120.0).abs() < 0.01);
}

#[test]
fn a_very_short_loop_is_guessed_as_fewer_beats_than_a_bar() {
    // Half a second: one beat at 120, rather than a bar at 480.
    let tempo = LoopTempo::guess(secs(0.5), RATE, 4);
    assert_eq!(tempo.beats(), 1);
}

#[test]
fn every_guess_sits_inside_the_guess_range() {
    // From half a second: anything shorter is past 140 even as a single
    // beat, and there's nothing smaller to guess.
    for tenths in 5..400 {
        let tempo = LoopTempo::guess(secs(f64::from(tenths) / 10.0), RATE, 4);
        assert!(
            GUESS_BPM.contains(&tempo.bpm()),
            "{tenths}: {}",
            tempo.bpm()
        );
    }
}

#[test]
fn a_loop_played_to_the_click_keeps_its_bars() {
    // Three bars at 120 - the guess would have said four at 160 or two
    // at 80; the click already said three.
    let bar = 4.0 * 60.0 * f64::from(RATE) / 120.0;
    let tempo = LoopTempo::from_bars((3.0 * bar) as usize, RATE, bar, 4);
    assert_eq!(tempo.beats(), 12);
    assert!((tempo.bpm() - 120.0).abs() < 0.001);
}

#[test]
fn halving_and_doubling_keep_the_beats_fitting_the_loop() {
    let tempo = LoopTempo::guess(secs(2.0), RATE, 4);
    assert_eq!((tempo.beats(), tempo.bpm()), (4, 120.0));

    let halved = tempo.halved(RANGE).expect("60 is in range");
    assert_eq!((halved.beats(), halved.bpm()), (2, 60.0));
    let doubled = tempo.doubled(RANGE).expect("240 is in range");
    assert_eq!((doubled.beats(), doubled.bpm()), (8, 240.0));
}

#[test]
fn an_odd_beat_count_cannot_be_halved() {
    // Three beats into one and a half: the click would miss the top.
    let bar = 4.0 * 60.0 * f64::from(RATE) / 120.0;
    let tempo = LoopTempo::from_bars((3.0 * bar / 4.0) as usize, RATE, bar / 4.0, 1);
    assert_eq!(tempo.beats(), 3);
    assert!(tempo.halved(RANGE).is_none());
}

#[test]
fn halving_and_doubling_stop_at_the_range() {
    let tempo = LoopTempo::guess(secs(2.0), RATE, 4);
    assert!(tempo.doubled(40.0..=200.0).is_none(), "240 is too fast");
    assert!(tempo.halved(70.0..=300.0).is_none(), "60 is too slow");
}

#[test]
fn a_guess_counts_in_bars_of_the_time_signature() {
    // Two bars of 3/4 at 100 BPM: six beats, where powers of two would
    // have said four or eight.
    let tempo = LoopTempo::guess(secs(3.6), RATE, 3);
    assert_eq!(tempo.beats(), 6);
    assert!((tempo.bpm() - 100.0).abs() < 0.01, "got {}", tempo.bpm());
}

#[test]
fn a_loop_shorter_than_a_bar_guesses_fewer_beats() {
    // A second of 4/4 would be 240 BPM as a whole bar: two beats instead.
    let tempo = LoopTempo::guess(secs(1.0), RATE, 4);
    assert_eq!((tempo.beats(), tempo.bpm()), (2, 120.0));
}
