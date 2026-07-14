use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DesktopId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Occupancy {
    Occupied,
    Empty,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopState {
    pub id: DesktopId,
    pub occupancy: Occupancy,
    pub current: bool,
    pub empty_grace_elapsed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutation {
    CreateTrailing,
    Remove { desktop: DesktopId, fallback: DesktopId },
    Switch { desktop: DesktopId },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    pub mutations: Vec<Mutation>,
    pub stable: bool,
}

pub fn plan(desktops: &[DesktopState]) -> Plan {
    if desktops.is_empty() {
        return Plan { mutations: vec![Mutation::CreateTrailing], stable: false };
    }

    let last = desktops.len() - 1;
    if desktops[last].occupancy == Occupancy::Occupied {
        return Plan { mutations: vec![Mutation::CreateTrailing], stable: false };
    }

    if desktops[last].occupancy == Occupancy::Unknown {
        return Plan { mutations: Vec::new(), stable: false };
    }

    let mut mutations = Vec::new();

    let trailing_empty_start = desktops
        .iter()
        .rposition(|desktop| desktop.occupancy != Occupancy::Empty)
        .map_or(0, |index| index + 1);
    if last.saturating_sub(trailing_empty_start) >= 1 {
        for index in (trailing_empty_start + 1..=last).rev() {
            let fallback = desktops[index - 1].id.clone();
            mutations.push(Mutation::Remove {
                desktop: desktops[index].id.clone(),
                fallback,
            });
        }
        return Plan { mutations, stable: false };
    }

    for (index, desktop) in desktops.iter().enumerate().take(last) {
        if desktop.occupancy != Occupancy::Empty || !desktop.empty_grace_elapsed {
            continue;
        }
        let fallback_index = nearest_safe_fallback(desktops, index);
        let Some(fallback_index) = fallback_index else { continue };
        if desktop.current {
            mutations.push(Mutation::Switch { desktop: desktops[fallback_index].id.clone() });
        }
        mutations.push(Mutation::Remove {
            desktop: desktop.id.clone(),
            fallback: desktops[fallback_index].id.clone(),
        });
        return Plan { mutations, stable: false };
    }

    Plan { mutations, stable: true }
}

fn nearest_safe_fallback(desktops: &[DesktopState], removing: usize) -> Option<usize> {
    let right = (removing + 1..desktops.len())
        .find(|&index| desktops[index].occupancy != Occupancy::Unknown);
    let left = (0..removing)
        .rev()
        .find(|&index| desktops[index].occupancy != Occupancy::Unknown);
    match (left, right) {
        (Some(left), Some(right)) => {
            if removing - left <= right - removing { Some(left) } else { Some(right) }
        }
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}
