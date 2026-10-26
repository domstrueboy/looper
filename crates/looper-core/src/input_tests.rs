use super::*;

/// The hold time is configurable now; these tests pin one so the timings
/// below mean something.
const THRESHOLD: Duration = Duration::from_millis(2000);

fn handler() -> InputHandler {
    InputHandler::new(THRESHOLD)
}

#[test]
fn a_press_fires_the_moment_the_control_goes_down() {
    // Not on release: that made every take start and stop late by
    // however long the key was held.
    let mut input = handler();
    let t0 = Instant::now();

    assert_eq!(input.update(true, t0), InputEvent::Press);
    assert_eq!(
        input.update(true, t0 + Duration::from_millis(100)),
        InputEvent::None,
        "once per press, not every frame it's held"
    );
    assert_eq!(
        input.update(false, t0 + Duration::from_millis(150)),
        InputEvent::None,
        "and nothing more on release"
    );
}

#[test]
fn holding_past_threshold_yields_long_press_clear_while_still_held() {
    let mut input = handler();
    let t0 = Instant::now();

    assert_eq!(input.update(true, t0), InputEvent::Press);
    assert_eq!(
        input.update(true, t0 + Duration::from_millis(2001)),
        InputEvent::LongPressClear
    );
}

#[test]
fn releasing_after_long_press_fires_nothing() {
    let mut input = handler();
    let t0 = Instant::now();

    input.update(true, t0);
    assert_eq!(
        input.update(true, t0 + Duration::from_millis(2001)),
        InputEvent::LongPressClear
    );
    assert_eq!(
        input.update(false, t0 + Duration::from_millis(2100)),
        InputEvent::None
    );
}

#[test]
fn long_press_fires_only_once_per_hold() {
    let mut input = handler();
    let t0 = Instant::now();

    input.update(true, t0);
    assert_eq!(
        input.update(true, t0 + Duration::from_millis(2001)),
        InputEvent::LongPressClear
    );
    assert_eq!(
        input.update(true, t0 + Duration::from_millis(3000)),
        InputEvent::None
    );
}

#[test]
fn releasing_without_ever_pressing_yields_none() {
    let mut input = handler();
    let t0 = Instant::now();
    assert_eq!(input.update(false, t0), InputEvent::None);
}

#[test]
fn can_press_again_after_a_press() {
    let mut input = handler();
    let t0 = Instant::now();

    assert_eq!(input.update(true, t0), InputEvent::Press);
    input.update(false, t0 + Duration::from_millis(100));

    let t1 = t0 + Duration::from_millis(500);
    assert_eq!(input.update(true, t1), InputEvent::Press);
}

#[test]
fn a_hold_cut_short_of_the_threshold_does_not_clear() {
    let mut input = handler();
    let t0 = Instant::now();

    input.update(true, t0);
    input.update(true, t0 + Duration::from_millis(1999));
    assert_eq!(
        input.update(false, t0 + Duration::from_millis(2001)),
        InputEvent::None,
        "released in time"
    );
}

#[test]
fn is_long_press_active_only_between_firing_and_release() {
    let mut input = handler();
    let t0 = Instant::now();

    input.update(true, t0);
    assert!(!input.is_long_press_active());

    input.update(true, t0 + Duration::from_millis(2001));
    assert!(input.is_long_press_active());

    input.update(true, t0 + Duration::from_millis(2500));
    assert!(input.is_long_press_active());

    input.update(false, t0 + Duration::from_millis(2600));
    assert!(!input.is_long_press_active());
}

#[test]
fn can_press_again_after_a_long_press_clear() {
    let mut input = handler();
    let t0 = Instant::now();

    input.update(true, t0);
    assert_eq!(
        input.update(true, t0 + Duration::from_millis(2001)),
        InputEvent::LongPressClear
    );
    input.update(false, t0 + Duration::from_millis(2100));

    let t1 = t0 + Duration::from_secs(3);
    assert_eq!(input.update(true, t1), InputEvent::Press);
}

#[test]
fn a_second_press_soon_after_is_a_double() {
    let mut input = handler();
    let t0 = Instant::now();

    assert_eq!(input.update(true, t0), InputEvent::Press, "acted on at once");
    input.update(false, t0 + Duration::from_millis(80));
    assert_eq!(
        input.update(true, t0 + Duration::from_millis(250)),
        InputEvent::DoublePress
    );
}

#[test]
fn a_second_press_after_the_window_is_a_single() {
    let mut input = handler();
    let t0 = Instant::now();

    input.update(true, t0);
    input.update(false, t0 + Duration::from_millis(80));
    assert_eq!(
        input.update(true, t0 + DOUBLE_PRESS_WINDOW + Duration::from_millis(1)),
        InputEvent::Press
    );
}

#[test]
fn three_quick_presses_are_a_double_then_a_single() {
    // Not two doubles sharing the middle press.
    let mut input = handler();
    let t0 = Instant::now();
    let ms = Duration::from_millis;

    assert_eq!(input.update(true, t0), InputEvent::Press);
    input.update(false, t0 + ms(50));
    assert_eq!(input.update(true, t0 + ms(150)), InputEvent::DoublePress);
    input.update(false, t0 + ms(200));
    assert_eq!(input.update(true, t0 + ms(300)), InputEvent::Press);
}
