// File purpose: Protects the trailing empty desktop until persistent eligible-window evidence proves that it was consumed.
use super::{DesktopId, DesktopState, Occupancy};
use std::collections::{HashMap, HashSet};

pub type WindowToken = u64;
const EVENT_CONFIRMATIONS: u8 = 2;
const PASSIVE_CONFIRMATIONS: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpareGuardResult {
    pub protecting: bool,
    pub consumed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SpareGuard {
    protected: Option<DesktopId>,
    observed_streaks: HashMap<WindowToken, u8>,
}

impl SpareGuard {
    // Function purpose: Protects a desktop selected as the trailing navigation spare even if transient shell windows make it look occupied.
    pub fn arm(&mut self, desktop: DesktopId) {
        if self.protected.as_ref() != Some(&desktop) {
            self.observed_streaks.clear();
        }
        self.protected = Some(desktop);
    }

    // Function purpose: Returns the desktop currently treated as the protected empty spare.
    pub fn protected(&self) -> Option<&DesktopId> {
        self.protected.as_ref()
    }

    // Function purpose: Reports whether eligible window evidence needs another observation before it can consume the spare.
    pub fn needs_confirmation(&self) -> bool {
        !self.observed_streaks.is_empty()
    }

    // Function purpose: Overrides raw occupancy until one eligible window persists long enough to prove that the protected spare contains a user application.
    pub fn stabilize(
        &mut self,
        states: &mut [DesktopState],
        confirmable_windows: &HashMap<DesktopId, HashSet<WindowToken>>,
        occupancy_gain_candidates: &HashSet<WindowToken>,
    ) -> SpareGuardResult {
        let Some(last) = states.last() else {
            self.clear();
            return SpareGuardResult::default();
        };
        let last_id = last.id.clone();

        if self.protected.as_ref().is_some_and(|id| id != &last_id) {
            self.clear();
        }
        if self.protected.is_none() && last.occupancy == Occupancy::Empty {
            self.protected = Some(last_id.clone());
        }

        let Some(protected) = self.protected.clone() else {
            return SpareGuardResult::default();
        };
        let observed: HashSet<_> = confirmable_windows
            .get(&protected)
            .into_iter()
            .flat_map(|tokens| tokens.iter().copied())
            .collect();
        self.observed_streaks
            .retain(|token, _| observed.contains(token));
        for token in &observed {
            let streak = self.observed_streaks.entry(*token).or_insert(0);
            *streak = streak.saturating_add(1);
        }

        let consumed = self.observed_streaks.iter().any(|(token, streak)| {
            let required = if occupancy_gain_candidates.contains(token) {
                EVENT_CONFIRMATIONS
            } else {
                PASSIVE_CONFIRMATIONS
            };
            *streak >= required
        });
        if consumed {
            self.clear();
            return SpareGuardResult {
                protecting: false,
                consumed: true,
            };
        }

        if let Some(state) = states.iter_mut().find(|state| state.id == protected) {
            state.occupancy = Occupancy::Empty;
            state.empty_grace_elapsed = false;
            SpareGuardResult {
                protecting: true,
                consumed: false,
            }
        } else {
            self.clear();
            SpareGuardResult::default()
        }
    }

    // Function purpose: Clears protection and all pending evidence when the spare changes or is consumed.
    fn clear(&mut self) {
        self.protected = None;
        self.observed_streaks.clear();
    }
}
