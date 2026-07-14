use crate::config::{NavigationMode, WheelDirection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Previous,
    Next,
}

#[derive(Debug, Clone)]
pub struct WheelState {
    accumulator: i32,
    last_step_ms: Option<u64>,
}

impl Default for WheelState {
    fn default() -> Self {
        Self { accumulator: 0, last_step_ms: None }
    }
}

impl WheelState {
    pub fn feed(
        &mut self,
        delta: i32,
        now_ms: u64,
        threshold: i32,
        cooldown_ms: u64,
        direction: WheelDirection,
    ) -> Option<Step> {
        if threshold <= 0 || delta == 0 {
            return None;
        }
        self.accumulator = self.accumulator.saturating_add(delta);
        if self.accumulator.unsigned_abs() < threshold.unsigned_abs() {
            return None;
        }
        if self
            .last_step_ms
            .is_some_and(|last| now_ms.saturating_sub(last) < cooldown_ms)
        {
            self.accumulator = self.accumulator.clamp(-threshold + 1, threshold - 1);
            return None;
        }
        let positive = self.accumulator > 0;
        self.accumulator = 0;
        self.last_step_ms = Some(now_ms);
        Some(match (positive, direction) {
            (true, WheelDirection::Normal) | (false, WheelDirection::Inverted) => Step::Previous,
            (false, WheelDirection::Normal) | (true, WheelDirection::Inverted) => Step::Next,
        })
    }

    pub fn reset(&mut self) {
        self.accumulator = 0;
    }
}

pub fn target_index(
    current: usize,
    count: usize,
    step: Step,
    mode: NavigationMode,
) -> Option<usize> {
    if count == 0 || current >= count {
        return None;
    }
    match (step, mode) {
        (Step::Previous, NavigationMode::Clamp) => current.checked_sub(1),
        (Step::Next, NavigationMode::Clamp) => (current + 1 < count).then_some(current + 1),
        (Step::Previous, NavigationMode::Wrap) => Some(if current == 0 { count - 1 } else { current - 1 }),
        (Step::Next, NavigationMode::Wrap) => Some((current + 1) % count),
    }
}
