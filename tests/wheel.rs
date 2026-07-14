// File purpose: Verifies wheel accumulation, cooldown, direction, clamp, and wrap navigation behavior.
use deskpilot::config::{NavigationMode, WheelDirection};
use deskpilot::wheel::{target_index, Step, WheelState};

// Function purpose: Verifies the standard wheel generates one step scenario and its expected safety or state invariant.
#[test]
fn standard_wheel_generates_one_step() {
    let mut state = WheelState::default();
    assert_eq!(
        state.feed(120, 1_000, 120, 180, WheelDirection::Normal),
        Some(Step::Previous)
    );
}

// Function purpose: Verifies the high resolution deltas accumulate scenario and its expected safety or state invariant.
#[test]
fn high_resolution_deltas_accumulate() {
    let mut state = WheelState::default();
    assert_eq!(
        state.feed(30, 1_000, 120, 180, WheelDirection::Normal),
        None
    );
    assert_eq!(
        state.feed(30, 1_010, 120, 180, WheelDirection::Normal),
        None
    );
    assert_eq!(
        state.feed(30, 1_020, 120, 180, WheelDirection::Normal),
        None
    );
    assert_eq!(
        state.feed(30, 1_030, 120, 180, WheelDirection::Normal),
        Some(Step::Previous)
    );
}

// Function purpose: Verifies the inverted direction reverses steps scenario and its expected safety or state invariant.
#[test]
fn inverted_direction_reverses_steps() {
    let mut state = WheelState::default();
    assert_eq!(
        state.feed(120, 1_000, 120, 0, WheelDirection::Inverted),
        Some(Step::Next)
    );
}

// Function purpose: Verifies the cooldown suppresses repeated switches scenario and its expected safety or state invariant.
#[test]
fn cooldown_suppresses_repeated_switches() {
    let mut state = WheelState::default();
    assert_eq!(
        state.feed(-120, 1_000, 120, 180, WheelDirection::Normal),
        Some(Step::Next)
    );
    assert_eq!(
        state.feed(-120, 1_050, 120, 180, WheelDirection::Normal),
        None
    );
    assert_eq!(
        state.feed(-120, 1_200, 120, 180, WheelDirection::Normal),
        Some(Step::Next)
    );
}

// Function purpose: Verifies the clamp stops at edges scenario and its expected safety or state invariant.
#[test]
fn clamp_stops_at_edges() {
    assert_eq!(
        target_index(0, 3, Step::Previous, NavigationMode::Clamp),
        None
    );
    assert_eq!(target_index(2, 3, Step::Next, NavigationMode::Clamp), None);
}

// Function purpose: Verifies the wrap cycles at edges scenario and its expected safety or state invariant.
#[test]
fn wrap_cycles_at_edges() {
    assert_eq!(
        target_index(0, 3, Step::Previous, NavigationMode::Wrap),
        Some(2)
    );
    assert_eq!(
        target_index(2, 3, Step::Next, NavigationMode::Wrap),
        Some(0)
    );
}

// Function purpose: Verifies the reset discards partial delta scenario and its expected safety or state invariant.
#[test]
fn reset_discards_partial_delta() {
    let mut state = WheelState::default();
    assert_eq!(state.feed(60, 1_000, 120, 0, WheelDirection::Normal), None);
    state.reset();
    assert_eq!(state.feed(60, 1_010, 120, 0, WheelDirection::Normal), None);
}
