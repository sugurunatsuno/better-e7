use std::{error::Error, fmt};

use better_e7_core::{Detection, Frame, InputCommand};

use crate::{StateId, TaskId, TriggerId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameState {
    current: StateId,
    previous: Option<StateId>,
    revision: u64,
}

impl GameState {
    #[must_use]
    pub const fn new(initial: StateId) -> Self {
        Self {
            current: initial,
            previous: None,
            revision: 0,
        }
    }

    #[must_use]
    pub const fn current(&self) -> &StateId {
        &self.current
    }

    #[must_use]
    pub const fn previous(&self) -> Option<&StateId> {
        self.previous.as_ref()
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn transition(&mut self, next: StateId) -> Option<StateTransition> {
        if self.current == next {
            return None;
        }
        let from = self.current.clone();
        self.previous = Some(from.clone());
        self.current = next.clone();
        self.revision = self.revision.saturating_add(1);
        Some(StateTransition { from, to: next })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateTransition {
    pub from: StateId,
    pub to: StateId,
}

pub struct GameContext<'a> {
    frame: &'a Frame,
    detections: &'a [Detection],
    state: &'a GameState,
}

impl<'a> GameContext<'a> {
    #[must_use]
    pub const fn new(frame: &'a Frame, detections: &'a [Detection], state: &'a GameState) -> Self {
        Self {
            frame,
            detections,
            state,
        }
    }

    #[must_use]
    pub const fn frame(&self) -> &Frame {
        self.frame
    }

    #[must_use]
    pub const fn detections(&self) -> &[Detection] {
        self.detections
    }

    #[must_use]
    pub const fn state(&self) -> &GameState {
        self.state
    }

    #[must_use]
    pub fn best_detection(&self, label: &str, minimum_confidence: f32) -> Option<&Detection> {
        self.detections
            .iter()
            .filter(|detection| {
                detection.label == label && detection.confidence >= minimum_confidence
            })
            .max_by(|left, right| left.confidence.total_cmp(&right.confidence))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputIntent {
    pub command: InputCommand,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DispatchPlan {
    pub(crate) input: Option<InputIntent>,
    pub(crate) transition: Option<StateId>,
    pub(crate) logs: Vec<String>,
}

impl DispatchPlan {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            input: None,
            transition: None,
            logs: Vec::new(),
        }
    }

    pub fn set_input(
        &mut self,
        command: InputCommand,
        reason: impl Into<String>,
    ) -> Result<(), PlanError> {
        if self.input.is_some() {
            return Err(PlanError::InputAlreadySet);
        }
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(PlanError::EmptyInputReason);
        }
        self.input = Some(InputIntent { command, reason });
        Ok(())
    }

    pub fn set_transition(&mut self, next: StateId) -> Result<(), PlanError> {
        if self.transition.is_some() {
            return Err(PlanError::TransitionAlreadySet);
        }
        self.transition = Some(next);
        Ok(())
    }

    pub fn log(&mut self, message: impl Into<String>) {
        let message = message.into();
        if !message.trim().is_empty() {
            self.logs.push(message);
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.input.is_none() && self.transition.is_none() && self.logs.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerFlow {
    Continue,
    Consume,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TriggerOutcome {
    Ignore,
    Match {
        flow: TriggerFlow,
        plan: DispatchPlan,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskStep {
    Running(DispatchPlan),
    Complete(DispatchPlan),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTaskState {
    Running,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskEvent {
    Started(TaskId),
    Paused(TaskId),
    Resumed(TaskId),
    Stopped(TaskId),
    Completed(TaskId),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DispatchReport {
    pub input: Option<InputIntent>,
    pub logs: Vec<String>,
    pub fired_triggers: Vec<TriggerId>,
    pub transition: Option<StateTransition>,
    pub task_event: Option<TaskEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    InputAlreadySet,
    EmptyInputReason,
    TransitionAlreadySet,
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputAlreadySet => formatter.write_str("dispatch plan already contains input"),
            Self::EmptyInputReason => formatter.write_str("input reason must not be empty"),
            Self::TransitionAlreadySet => {
                formatter.write_str("dispatch plan already contains a state transition")
            }
        }
    }
}

impl Error for PlanError {}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use better_e7_core::{NormalizedPoint, NormalizedRect, PixelFormat};

    use super::*;

    #[test]
    fn selects_the_best_detection_above_the_threshold() {
        let frame = Frame::new(1, Instant::now(), 1, 1, PixelFormat::Rgb8, vec![0; 3]).unwrap();
        let bounds = NormalizedRect::new(0.1, 0.1, 0.2, 0.2).unwrap();
        let detections = [
            Detection {
                label: "confirm".to_owned(),
                confidence: 0.91,
                center: NormalizedPoint::new(0.15, 0.15).unwrap(),
                bounds,
            },
            Detection {
                label: "confirm".to_owned(),
                confidence: 0.97,
                center: NormalizedPoint::new(0.15, 0.15).unwrap(),
                bounds,
            },
        ];
        let state = GameState::new(StateId::new("home").unwrap());
        let context = GameContext::new(&frame, &detections, &state);

        assert_eq!(
            context.best_detection("confirm", 0.9).unwrap().confidence,
            0.97
        );
        assert!(context.best_detection("confirm", 0.98).is_none());
    }

    #[test]
    fn keeps_one_input_and_one_transition_per_plan() {
        let mut plan = DispatchPlan::new();
        plan.set_input(
            InputCommand::Key {
                android_key_code: 3,
            },
            "return home",
        )
        .unwrap();
        plan.set_transition(StateId::new("home").unwrap()).unwrap();

        assert_eq!(
            plan.set_input(
                InputCommand::Key {
                    android_key_code: 4
                },
                "second input"
            ),
            Err(PlanError::InputAlreadySet)
        );
        assert_eq!(
            plan.set_transition(StateId::new("battle").unwrap()),
            Err(PlanError::TransitionAlreadySet)
        );
    }
}
