use super::*;

#[test]
fn starts_idle() {
    let sm = LoopStateMachine::new();
    assert_eq!(sm.state(), LoopState::Idle);
}

#[test]
fn press_cycles_through_full_sequence() {
    let mut sm = LoopStateMachine::new();

    sm.press();
    assert_eq!(sm.state(), LoopState::Recording);

    sm.press();
    assert_eq!(sm.state(), LoopState::Looping);

    sm.press();
    assert_eq!(sm.state(), LoopState::Stopped);

    sm.press();
    assert_eq!(sm.state(), LoopState::Looping);

    sm.press();
    assert_eq!(sm.state(), LoopState::Stopped);
}

#[test]
fn clear_from_idle_stays_idle() {
    let mut sm = LoopStateMachine::new();
    sm.clear();
    assert_eq!(sm.state(), LoopState::Idle);
}

#[test]
fn clear_from_recording_returns_to_idle() {
    let mut sm = LoopStateMachine::new();
    sm.press();
    assert_eq!(sm.state(), LoopState::Recording);
    sm.clear();
    assert_eq!(sm.state(), LoopState::Idle);
}

#[test]
fn clear_from_looping_returns_to_idle() {
    let mut sm = LoopStateMachine::new();
    sm.press();
    sm.press();
    assert_eq!(sm.state(), LoopState::Looping);
    sm.clear();
    assert_eq!(sm.state(), LoopState::Idle);
}

#[test]
fn clear_from_stopped_returns_to_idle() {
    let mut sm = LoopStateMachine::new();
    sm.press();
    sm.press();
    sm.press();
    assert_eq!(sm.state(), LoopState::Stopped);
    sm.clear();
    assert_eq!(sm.state(), LoopState::Idle);
}

#[test]
fn arming_counts_down_from_idle_then_records() {
    let mut sm = LoopStateMachine::new();

    sm.arm();
    assert_eq!(sm.state(), LoopState::Arming);

    sm.finish_arming();
    assert_eq!(sm.state(), LoopState::Recording);
}

#[test]
fn arming_only_applies_from_idle() {
    for presses in [1, 2, 3] {
        let mut sm = LoopStateMachine::new();
        for _ in 0..presses {
            sm.press();
        }
        let before = sm.state();
        sm.arm();
        assert_eq!(sm.state(), before, "after {presses} press(es)");
    }
}

#[test]
fn press_while_arming_calls_the_pre_roll_off() {
    let mut sm = LoopStateMachine::new();
    sm.arm();
    sm.press();
    assert_eq!(sm.state(), LoopState::Idle);
}

#[test]
fn clear_from_arming_returns_to_idle() {
    let mut sm = LoopStateMachine::new();
    sm.arm();
    sm.clear();
    assert_eq!(sm.state(), LoopState::Idle);
}

#[test]
fn finish_arming_does_nothing_if_the_pre_roll_was_called_off() {
    let mut sm = LoopStateMachine::new();
    sm.arm();
    sm.press();
    sm.finish_arming();
    assert_eq!(sm.state(), LoopState::Idle);
}

#[test]
fn toggle_overdub_starts_and_ends_a_layer_while_looping() {
    let mut sm = LoopStateMachine::new();
    sm.press();
    sm.press();
    assert_eq!(sm.state(), LoopState::Looping);

    sm.toggle_overdub();
    assert_eq!(sm.state(), LoopState::Overdubbing);

    sm.toggle_overdub();
    assert_eq!(sm.state(), LoopState::Looping);
}

#[test]
fn toggle_overdub_does_nothing_without_a_playing_loop() {
    for presses in [0, 1, 3] {
        let mut sm = LoopStateMachine::new();
        for _ in 0..presses {
            sm.press();
        }
        let before = sm.state();
        sm.toggle_overdub();
        assert_eq!(sm.state(), before);
    }
}

#[test]
fn press_while_overdubbing_stops_playback() {
    let mut sm = LoopStateMachine::new();
    sm.press();
    sm.press();
    sm.toggle_overdub();
    sm.press();
    assert_eq!(sm.state(), LoopState::Stopped);
}

#[test]
fn clear_from_overdubbing_returns_to_idle() {
    let mut sm = LoopStateMachine::new();
    sm.press();
    sm.press();
    sm.toggle_overdub();
    sm.clear();
    assert_eq!(sm.state(), LoopState::Idle);
}

#[test]
fn can_record_again_after_clear() {
    let mut sm = LoopStateMachine::new();
    sm.press();
    sm.press();
    sm.clear();
    sm.press();
    assert_eq!(sm.state(), LoopState::Recording);
}
