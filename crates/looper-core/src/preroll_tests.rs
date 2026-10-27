use super::*;

/// The configured wait these tests count down; the timings below are
/// relative to it.
const WAIT: Duration = Duration::from_secs(5);

fn preroll() -> Preroll {
    Preroll::new(WAIT)
}

#[test]
fn nothing_fires_until_a_countdown_is_started() {
    let mut preroll = preroll();
    let t0 = Instant::now();
    assert!(!preroll.take_if_elapsed(t0 + Duration::from_secs(60)));
    assert_eq!(preroll.remaining_secs(t0), 0.0);
    assert_eq!(preroll.progress(t0), 0.0);
}

#[test]
fn fires_once_when_the_wait_runs_out() {
    let mut preroll = preroll();
    let t0 = Instant::now();
    preroll.start(t0);

    assert!(
        !preroll.take_if_elapsed(t0 + Duration::from_secs(4)),
        "early"
    );
    assert!(preroll.take_if_elapsed(t0 + WAIT));
    // Recording has started; firing again would restart it.
    assert!(!preroll.take_if_elapsed(t0 + Duration::from_secs(6)));
}

#[test]
fn a_countdown_called_off_never_fires() {
    let mut preroll = preroll();
    let t0 = Instant::now();
    preroll.start(t0);
    preroll.cancel();

    // The press that cancelled it is handled before the countdown is
    // checked, so this is the frame the wait would have run out on.
    assert!(!preroll.take_if_elapsed(t0 + WAIT));
}

#[test]
fn a_countdown_can_be_started_again_after_being_called_off() {
    let mut preroll = preroll();
    let t0 = Instant::now();
    preroll.start(t0);
    preroll.cancel();

    let t1 = t0 + Duration::from_secs(10);
    preroll.start(t1);
    assert!(preroll.take_if_elapsed(t1 + WAIT));
}

#[test]
fn the_readout_counts_down_and_stops_at_zero() {
    let mut preroll = preroll();
    let t0 = Instant::now();
    preroll.start(t0);

    assert_eq!(preroll.remaining_secs(t0), 5.0);
    assert_eq!(
        preroll.remaining_secs(t0 + Duration::from_millis(1500)),
        3.5
    );
    // A frame landing past the end must not read as negative.
    assert_eq!(preroll.remaining_secs(t0 + Duration::from_secs(9)), 0.0);
}

#[test]
fn the_bar_fills_from_zero_to_one() {
    let mut preroll = preroll();
    let t0 = Instant::now();
    preroll.start(t0);

    assert_eq!(preroll.progress(t0), 0.0);
    assert_eq!(preroll.progress(t0 + Duration::from_millis(2500)), 0.5);
    assert_eq!(
        preroll.progress(t0 + Duration::from_secs(9)),
        1.0,
        "clamped"
    );
}

#[test]
fn a_wait_of_zero_is_off_rather_than_instant() {
    let mut preroll = Preroll::new(Duration::ZERO);
    let t0 = Instant::now();
    assert!(!preroll.is_enabled(), "the caller records straight away");

    // And if one is started anyway, the bar must not divide by zero.
    preroll.start(t0);
    assert_eq!(preroll.progress(t0), 0.0);
    assert!(preroll.take_if_elapsed(t0));
}
