// File purpose: Models desktop occupancy and computes safe create or remove mutations.
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
    Remove {
        desktop: DesktopId,
        fallback: DesktopId,
    },
    Switch {
        desktop: DesktopId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    pub mutations: Vec<Mutation>,
    pub stable: bool,
}

// Function purpose: Computes the next safe desktop mutation required to restore the dynamic desktop invariant.
pub fn plan(desktops: &[DesktopState]) -> Plan {
    if desktops.is_empty() {
        return Plan {
            mutations: vec![Mutation::CreateTrailing],
            stable: false,
        };
    }

    let last = desktops.len() - 1;
    if desktops[last].occupancy == Occupancy::Occupied {
        return Plan {
            mutations: vec![Mutation::CreateTrailing],
            stable: false,
        };
    }

    if desktops[last].occupancy == Occupancy::Unknown {
        return Plan {
            mutations: Vec::new(),
            stable: false,
        };
    }

    let trailing_empty_start = desktops
        .iter()
        .rposition(|desktop| desktop.occupancy != Occupancy::Empty)
        .map_or(0, |index| index + 1);
    let trailing_empty_count = desktops.len() - trailing_empty_start;

    if trailing_empty_count > 1 {
        if let Some(removing) = (trailing_empty_start..=last)
            .rev()
            .find(|&index| !desktops[index].current)
        {
            let fallback = trailing_empty_fallback(trailing_empty_start, removing);
            return Plan {
                mutations: vec![Mutation::Remove {
                    desktop: desktops[removing].id.clone(),
                    fallback: desktops[fallback].id.clone(),
                }],
                stable: false,
            };
        }
    }

    for desktop in desktops.iter().take(last) {
        if desktop.current || desktop.occupancy != Occupancy::Empty || !desktop.empty_grace_elapsed
        {
            continue;
        }

        return Plan {
            mutations: vec![Mutation::Remove {
                desktop: desktop.id.clone(),
                fallback: desktops[last].id.clone(),
            }],
            stable: false,
        };
    }

    Plan {
        mutations: Vec::new(),
        stable: true,
    }
}

// Function purpose: Performs the trailing empty fallback operation required by this module.
fn trailing_empty_fallback(trailing_empty_start: usize, removing: usize) -> usize {
    if removing > trailing_empty_start {
        removing - 1
    } else {
        removing + 1
    }
}
