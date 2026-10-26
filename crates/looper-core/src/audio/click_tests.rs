use super::*;

#[test]
fn its_length_follows_the_granted_rate() {
    // 20 ms either way; a shared-mode endpoint's rate is not a choice.
    assert_eq!(click(44_100).len(), 882);
    assert_eq!(click(48_000).len(), 960);
}

#[test]
fn it_starts_and_ends_silent() {
    // A burst that starts or stops mid-wave adds a pop of its own.
    let click = click(44_100);
    assert_eq!(click[0], 0.0);
    assert_eq!(*click.last().unwrap(), 0.0);
}

#[test]
fn it_stays_under_its_peak_and_actually_reaches_it() {
    let peak = click(48_000).iter().fold(0.0f32, |max, s| max.max(s.abs()));
    assert!(peak <= CLICK_PEAK, "got {peak}");
    assert!(peak > CLICK_PEAK * 0.9, "got {peak}");
}

#[test]
fn it_is_loudest_at_the_front() {
    // A tick, not a beep.
    let click = click(48_000);
    let (front, back) = click.split_at(click.len() / 2);
    let energy = |half: &[f32]| half.iter().map(|s| s * s).sum::<f32>();
    assert!(energy(front) > energy(back) * 4.0);
}

#[test]
fn it_has_the_pitch_it_claims() {
    // 1 kHz for 20 ms is 20 cycles, two zero crossings each.
    let click = click(48_000);
    let crossings = click
        .windows(2)
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count();
    assert!((38..=41).contains(&crossings), "got {crossings}");
}
