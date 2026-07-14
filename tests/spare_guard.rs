// File purpose: Reproduces repeated scroll visits to a noisy empty spare and proves only persistent visible event-confirmed user windows can consume it.
use deskpilot::reconciliation::{
    plan, DesktopId, DesktopState, Mutation, Occupancy, SpareGuard, WindowToken,
};
use std::collections::{HashMap, HashSet};

fn state(index: usize, occupancy: Occupancy, current: bool) -> DesktopState {
    DesktopState {
        id: DesktopId(format!("desktop-{index}")),
        occupancy,
        current,
        empty_grace_elapsed: true,
    }
}

// Function purpose: Verifies repeated scroll visits cannot convert switch-time shell noise into additional desktop creation.
#[test]
fn one_app_plus_repeated_scroll_stays_at_two_desktops() {
    let mut guard = SpareGuard::default();
    let spare = DesktopId("desktop-1".to_string());
    let candidates = HashSet::new();

    let mut initial = vec![
        state(0, Occupancy::Occupied, true),
        state(1, Occupancy::Empty, false),
    ];
    guard.stabilize(&mut initial, &HashMap::new(), &candidates);
    assert_eq!(guard.protected(), Some(&spare));

    for iteration in 0..200 {
        guard.arm(spare.clone());
        let mut noisy = vec![
            state(0, Occupancy::Occupied, iteration % 2 == 0),
            state(1, Occupancy::Occupied, iteration % 2 != 0),
        ];
        let visible_windows = HashMap::from([(spare.clone(), HashSet::from([9001]))]);
        let result = guard.stabilize(&mut noisy, &visible_windows, &candidates);
        assert!(result.protecting);
        assert!(!result.consumed);
        assert!(!guard.needs_confirmation());
        assert_eq!(noisy[1].occupancy, Occupancy::Empty);
        assert!(plan(&noisy).mutations.is_empty());
    }
}

// Function purpose: Verifies a visible qualifying window must survive two observations before it can authorize one new trailing desktop.
#[test]
fn persistent_real_window_event_consumes_spare_once() {
    let mut guard = SpareGuard::default();
    let spare = DesktopId("desktop-1".to_string());
    guard.arm(spare.clone());
    let token: WindowToken = 42;
    let visible_windows = HashMap::from([(spare.clone(), HashSet::from([token]))]);
    let candidates = HashSet::from([token]);
    let mut first = vec![
        state(0, Occupancy::Occupied, false),
        state(1, Occupancy::Occupied, true),
    ];

    let first_result = guard.stabilize(&mut first, &visible_windows, &candidates);
    assert!(first_result.protecting);
    assert!(!first_result.consumed);
    assert!(guard.needs_confirmation());
    assert!(plan(&first).mutations.is_empty());

    let mut second = vec![
        state(0, Occupancy::Occupied, false),
        state(1, Occupancy::Occupied, true),
    ];
    let second_result = guard.stabilize(&mut second, &visible_windows, &candidates);
    assert!(second_result.consumed);
    assert!(!second_result.protecting);
    assert_eq!(guard.protected(), None);
    assert!(!guard.needs_confirmation());
    assert_eq!(plan(&second).mutations, vec![Mutation::CreateTrailing]);
}

// Function purpose: Verifies a transient event-backed surface that disappears before confirmation cannot consume the spare.
#[test]
fn transient_event_surface_cannot_consume_spare() {
    let mut guard = SpareGuard::default();
    let spare = DesktopId("desktop-1".to_string());
    guard.arm(spare.clone());
    let token: WindowToken = 55;
    let candidates = HashSet::from([token]);
    let mut first = vec![
        state(0, Occupancy::Occupied, false),
        state(1, Occupancy::Occupied, true),
    ];
    let first_windows = HashMap::from([(spare.clone(), HashSet::from([token]))]);
    guard.stabilize(&mut first, &first_windows, &candidates);
    assert!(guard.needs_confirmation());

    let mut second = vec![
        state(0, Occupancy::Occupied, false),
        state(1, Occupancy::Occupied, true),
    ];
    let result = guard.stabilize(&mut second, &HashMap::new(), &candidates);
    assert!(result.protecting);
    assert!(!result.consumed);
    assert!(!guard.needs_confirmation());
    assert_eq!(second[1].occupancy, Occupancy::Empty);
    assert!(plan(&second).mutations.is_empty());
}

// Function purpose: Verifies unrelated application events on another desktop cannot consume the protected spare.
#[test]
fn unrelated_window_event_does_not_consume_spare() {
    let mut guard = SpareGuard::default();
    let spare = DesktopId("desktop-1".to_string());
    guard.arm(spare.clone());
    let mut states = vec![
        state(0, Occupancy::Occupied, true),
        state(1, Occupancy::Occupied, false),
    ];
    let visible_windows = HashMap::from([
        (DesktopId("desktop-0".to_string()), HashSet::from([77])),
        (spare.clone(), HashSet::from([9001])),
    ]);
    let candidates = HashSet::from([77]);

    let result = guard.stabilize(&mut states, &visible_windows, &candidates);
    assert!(result.protecting);
    assert!(!result.consumed);
    assert!(!guard.needs_confirmation());
    assert_eq!(states[1].occupancy, Occupancy::Empty);
    assert!(plan(&states).mutations.is_empty());
}

// Function purpose: Verifies the guard follows a newly created trailing spare and discards stale confirmation evidence.
#[test]
fn guard_moves_to_new_trailing_spare() {
    let mut guard = SpareGuard::default();
    guard.arm(DesktopId("desktop-1".to_string()));
    let mut states = vec![
        state(0, Occupancy::Occupied, true),
        state(1, Occupancy::Occupied, false),
        state(2, Occupancy::Empty, false),
    ];

    let result = guard.stabilize(&mut states, &HashMap::new(), &HashSet::new());
    assert!(result.protecting);
    assert_eq!(guard.protected(), Some(&DesktopId("desktop-2".to_string())));
    assert!(!guard.needs_confirmation());
    assert_eq!(states[2].occupancy, Occupancy::Empty);
}
