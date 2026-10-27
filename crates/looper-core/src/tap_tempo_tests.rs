use super::*;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn one_tap_is_no_tempo_yet() {
    let mut tap = TapTempo::new();
    assert_eq!(tap.tap(Instant::now()), None);
}

#[test]
fn two_taps_half_a_second_apart_are_120() {
    let mut tap = TapTempo::new();
    let t0 = Instant::now();
    tap.tap(t0);
    assert_eq!(tap.tap(t0 + ms(500)), Some(120.0));
}

#[test]
fn the_gaps_are_averaged() {
    // A frame early, a frame late: 480 and 520 average to 500.
    let mut tap = TapTempo::new();
    let t0 = Instant::now();
    tap.tap(t0);
    tap.tap(t0 + ms(480));
    assert_eq!(tap.tap(t0 + ms(1_000)), Some(120.0));
}

#[test]
fn only_the_last_few_gaps_count() {
    // Four slow gaps, then four at 120: the slow ones are forgotten.
    let mut tap = TapTempo::new();
    let mut at = Instant::now();
    tap.tap(at);
    for _ in 0..TAP_GAPS {
        at += ms(1_000);
        tap.tap(at);
    }
    let mut bpm = None;
    for _ in 0..TAP_GAPS {
        at += ms(500);
        bpm = tap.tap(at);
    }
    assert_eq!(bpm, Some(120.0));
}

#[test]
fn a_long_pause_starts_a_fresh_count() {
    let mut tap = TapTempo::new();
    let t0 = Instant::now();
    tap.tap(t0);
    tap.tap(t0 + ms(1_000));

    let later = t0 + ms(1_000) + TAP_TIMEOUT + ms(1);
    assert_eq!(tap.tap(later), None, "the first tap of a new count");
    assert_eq!(tap.tap(later + ms(500)), Some(120.0), "nothing of the old one left");
}
