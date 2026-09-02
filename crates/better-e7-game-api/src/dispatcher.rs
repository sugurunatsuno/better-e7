use std::{collections::BTreeMap, error::Error, fmt};

use better_e7_core::{Detection, Frame};

use crate::{
    ActiveTaskState, DispatchPlan, DispatchReport, GameContext, GameState, StateId, TaskEvent,
    TaskId, TaskStep, TriggerFlow, TriggerId, TriggerOutcome,
};

pub trait Trigger: Send {
    fn id(&self) -> &TriggerId;

    fn priority(&self) -> i32 {
        0
    }

    fn enabled(&self) -> bool {
        true
    }

    fn evaluate(&mut self, context: &GameContext<'_>) -> Result<TriggerOutcome, ComponentError>;
}

pub trait Task: Send {
    fn id(&self) -> &TaskId;

    fn start(&mut self, _context: &GameContext<'_>) -> Result<TaskStep, ComponentError> {
        Ok(TaskStep::Running(DispatchPlan::new()))
    }

    fn update(&mut self, context: &GameContext<'_>) -> Result<TaskStep, ComponentError>;

    fn stop(&mut self, _context: &GameContext<'_>) -> Result<DispatchPlan, ComponentError> {
        Ok(DispatchPlan::new())
    }
}

struct TriggerSlot {
    priority: i32,
    registration_order: u64,
    trigger: Box<dyn Trigger>,
}

struct ActiveTask {
    id: TaskId,
    state: ActiveTaskState,
}

pub struct Dispatcher {
    state: GameState,
    triggers: Vec<TriggerSlot>,
    tasks: BTreeMap<TaskId, Box<dyn Task>>,
    active_task: Option<ActiveTask>,
    next_registration_order: u64,
}

impl Dispatcher {
    #[must_use]
    pub const fn new(initial_state: StateId) -> Self {
        Self {
            state: GameState::new(initial_state),
            triggers: Vec::new(),
            tasks: BTreeMap::new(),
            active_task: None,
            next_registration_order: 0,
        }
    }

    #[must_use]
    pub const fn state(&self) -> &GameState {
        &self.state
    }

    #[must_use]
    pub fn active_task_id(&self) -> Option<&TaskId> {
        self.active_task.as_ref().map(|active| &active.id)
    }

    #[must_use]
    pub fn active_task_state(&self) -> Option<ActiveTaskState> {
        self.active_task.as_ref().map(|active| active.state)
    }

    pub fn register_trigger(
        &mut self,
        trigger: impl Trigger + 'static,
    ) -> Result<(), DispatcherError> {
        let id = trigger.id().clone();
        if self.triggers.iter().any(|slot| slot.trigger.id() == &id) {
            return Err(DispatcherError::DuplicateTrigger(id));
        }
        let slot = TriggerSlot {
            priority: trigger.priority(),
            registration_order: self.next_registration_order,
            trigger: Box::new(trigger),
        };
        self.next_registration_order = self.next_registration_order.saturating_add(1);
        self.triggers.push(slot);
        self.triggers.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then(left.registration_order.cmp(&right.registration_order))
        });
        Ok(())
    }

    pub fn register_task(&mut self, task: impl Task + 'static) -> Result<(), DispatcherError> {
        let id = task.id().clone();
        if self.tasks.contains_key(&id) {
            return Err(DispatcherError::DuplicateTask(id));
        }
        self.tasks.insert(id, Box::new(task));
        Ok(())
    }

    pub fn tick(
        &mut self,
        frame: &Frame,
        detections: &[Detection],
    ) -> Result<DispatchReport, DispatcherError> {
        let mut report = DispatchReport::default();
        let mut consumed = false;

        for index in 0..self.triggers.len() {
            let (id, outcome) = {
                let slot = &mut self.triggers[index];
                if !slot.trigger.enabled() {
                    continue;
                }
                let context = GameContext::new(frame, detections, &self.state);
                let id = slot.trigger.id().clone();
                let outcome = slot.trigger.evaluate(&context).map_err(|error| {
                    DispatcherError::TriggerFailed {
                        id: id.clone(),
                        error,
                    }
                })?;
                (id, outcome)
            };

            if let TriggerOutcome::Match { flow, plan } = outcome {
                self.apply_plan(plan, &mut report)?;
                report.fired_triggers.push(id);
                if flow == TriggerFlow::Consume {
                    consumed = true;
                    break;
                }
            }
        }

        if !consumed {
            self.update_active_task(frame, detections, &mut report)?;
        }
        Ok(report)
    }

    pub fn start_task(
        &mut self,
        id: &TaskId,
        frame: &Frame,
        detections: &[Detection],
    ) -> Result<DispatchReport, DispatcherError> {
        if let Some(active) = &self.active_task {
            return Err(DispatcherError::TaskAlreadyActive(active.id.clone()));
        }
        let step = {
            let task = self
                .tasks
                .get_mut(id)
                .ok_or_else(|| DispatcherError::UnknownTask(id.clone()))?;
            let context = GameContext::new(frame, detections, &self.state);
            task.start(&context)
                .map_err(|error| DispatcherError::TaskFailed {
                    id: id.clone(),
                    error,
                })?
        };
        let mut report = DispatchReport::default();
        match step {
            TaskStep::Running(plan) => {
                self.apply_plan(plan, &mut report)?;
                self.active_task = Some(ActiveTask {
                    id: id.clone(),
                    state: ActiveTaskState::Running,
                });
                report.task_event = Some(TaskEvent::Started(id.clone()));
            }
            TaskStep::Complete(plan) => {
                self.apply_plan(plan, &mut report)?;
                report.task_event = Some(TaskEvent::Completed(id.clone()));
            }
        }
        Ok(report)
    }

    pub fn pause_task(&mut self) -> Result<DispatchReport, DispatcherError> {
        let active = self
            .active_task
            .as_mut()
            .ok_or(DispatcherError::NoActiveTask)?;
        if active.state == ActiveTaskState::Paused {
            return Err(DispatcherError::TaskAlreadyPaused(active.id.clone()));
        }
        active.state = ActiveTaskState::Paused;
        Ok(DispatchReport {
            task_event: Some(TaskEvent::Paused(active.id.clone())),
            ..DispatchReport::default()
        })
    }

    pub fn resume_task(&mut self) -> Result<DispatchReport, DispatcherError> {
        let active = self
            .active_task
            .as_mut()
            .ok_or(DispatcherError::NoActiveTask)?;
        if active.state == ActiveTaskState::Running {
            return Err(DispatcherError::TaskAlreadyRunning(active.id.clone()));
        }
        active.state = ActiveTaskState::Running;
        Ok(DispatchReport {
            task_event: Some(TaskEvent::Resumed(active.id.clone())),
            ..DispatchReport::default()
        })
    }

    pub fn stop_task(
        &mut self,
        frame: &Frame,
        detections: &[Detection],
    ) -> Result<DispatchReport, DispatcherError> {
        let id = self
            .active_task
            .as_ref()
            .map(|active| active.id.clone())
            .ok_or(DispatcherError::NoActiveTask)?;
        let plan = {
            let task = self
                .tasks
                .get_mut(&id)
                .ok_or_else(|| DispatcherError::UnknownTask(id.clone()))?;
            let context = GameContext::new(frame, detections, &self.state);
            task.stop(&context)
                .map_err(|error| DispatcherError::TaskFailed {
                    id: id.clone(),
                    error,
                })?
        };
        let mut report = DispatchReport::default();
        self.apply_plan(plan, &mut report)?;
        self.active_task = None;
        report.task_event = Some(TaskEvent::Stopped(id));
        Ok(report)
    }

    fn update_active_task(
        &mut self,
        frame: &Frame,
        detections: &[Detection],
        report: &mut DispatchReport,
    ) -> Result<(), DispatcherError> {
        let Some(active) = &self.active_task else {
            return Ok(());
        };
        if active.state == ActiveTaskState::Paused {
            return Ok(());
        }
        let id = active.id.clone();
        let step = {
            let task = self
                .tasks
                .get_mut(&id)
                .ok_or_else(|| DispatcherError::UnknownTask(id.clone()))?;
            let context = GameContext::new(frame, detections, &self.state);
            task.update(&context)
                .map_err(|error| DispatcherError::TaskFailed {
                    id: id.clone(),
                    error,
                })?
        };
        match step {
            TaskStep::Running(plan) => self.apply_plan(plan, report)?,
            TaskStep::Complete(plan) => {
                self.apply_plan(plan, report)?;
                self.active_task = None;
                report.task_event = Some(TaskEvent::Completed(id));
            }
        }
        Ok(())
    }

    fn apply_plan(
        &mut self,
        plan: DispatchPlan,
        report: &mut DispatchReport,
    ) -> Result<(), DispatcherError> {
        if report.input.is_some() && plan.input.is_some() {
            return Err(DispatcherError::MultipleInputsInTick);
        }
        if report.transition.is_some() && plan.transition.is_some() {
            return Err(DispatcherError::MultipleTransitionsInTick);
        }
        if let Some(input) = plan.input {
            report.input = Some(input);
        }
        report.logs.extend(plan.logs);
        if let Some(next) = plan.transition {
            report.transition = self.state.transition(next);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentError(pub String);

impl fmt::Display for ComponentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ComponentError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatcherError {
    DuplicateTrigger(TriggerId),
    DuplicateTask(TaskId),
    UnknownTask(TaskId),
    TaskAlreadyActive(TaskId),
    NoActiveTask,
    TaskAlreadyPaused(TaskId),
    TaskAlreadyRunning(TaskId),
    MultipleInputsInTick,
    MultipleTransitionsInTick,
    TriggerFailed {
        id: TriggerId,
        error: ComponentError,
    },
    TaskFailed {
        id: TaskId,
        error: ComponentError,
    },
}

impl fmt::Display for DispatcherError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTrigger(id) => write!(formatter, "trigger is already registered: {id}"),
            Self::DuplicateTask(id) => write!(formatter, "task is already registered: {id}"),
            Self::UnknownTask(id) => write!(formatter, "task is not registered: {id}"),
            Self::TaskAlreadyActive(id) => write!(formatter, "task is already active: {id}"),
            Self::NoActiveTask => formatter.write_str("there is no active task"),
            Self::TaskAlreadyPaused(id) => write!(formatter, "task is already paused: {id}"),
            Self::TaskAlreadyRunning(id) => write!(formatter, "task is already running: {id}"),
            Self::MultipleInputsInTick => {
                formatter.write_str("a dispatch tick may emit at most one input")
            }
            Self::MultipleTransitionsInTick => {
                formatter.write_str("a dispatch tick may apply at most one state transition")
            }
            Self::TriggerFailed { id, error } => write!(formatter, "trigger failed: {id}: {error}"),
            Self::TaskFailed { id, error } => write!(formatter, "task failed: {id}: {error}"),
        }
    }
}

impl Error for DispatcherError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::TriggerFailed { error, .. } | Self::TaskFailed { error, .. } => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Instant,
    };

    use better_e7_core::{InputCommand, PixelFormat};

    use super::*;

    struct MockTrigger {
        id: TriggerId,
        priority: i32,
        flow: TriggerFlow,
        command: Option<InputCommand>,
        transition: Option<StateId>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl Trigger for MockTrigger {
        fn id(&self) -> &TriggerId {
            &self.id
        }

        fn priority(&self) -> i32 {
            self.priority
        }

        fn evaluate(
            &mut self,
            _context: &GameContext<'_>,
        ) -> Result<TriggerOutcome, ComponentError> {
            self.calls.lock().unwrap().push(self.id.to_string());
            let mut plan = DispatchPlan::new();
            if let Some(command) = self.command {
                plan.set_input(command, self.id.to_string()).unwrap();
            }
            if let Some(state) = &self.transition {
                plan.set_transition(state.clone()).unwrap();
            }
            Ok(TriggerOutcome::Match {
                flow: self.flow,
                plan,
            })
        }
    }

    struct CompletingTask {
        id: TaskId,
        updates: Arc<Mutex<u32>>,
    }

    impl Task for CompletingTask {
        fn id(&self) -> &TaskId {
            &self.id
        }

        fn update(&mut self, _context: &GameContext<'_>) -> Result<TaskStep, ComponentError> {
            let mut updates = self.updates.lock().unwrap();
            *updates += 1;
            let mut plan = DispatchPlan::new();
            plan.set_input(
                InputCommand::Key {
                    android_key_code: 4,
                },
                "task update",
            )
            .unwrap();
            Ok(TaskStep::Complete(plan))
        }
    }

    fn id(value: &str) -> StateId {
        StateId::new(value).unwrap()
    }

    fn frame() -> Frame {
        Frame::new(1, Instant::now(), 1, 1, PixelFormat::Rgb8, vec![0; 3]).unwrap()
    }

    fn trigger(
        id: &str,
        priority: i32,
        flow: TriggerFlow,
        command: Option<InputCommand>,
        calls: Arc<Mutex<Vec<String>>>,
    ) -> MockTrigger {
        MockTrigger {
            id: TriggerId::new(id).unwrap(),
            priority,
            flow,
            command,
            transition: None,
            calls,
        }
    }

    #[test]
    fn evaluates_consuming_triggers_by_priority() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut dispatcher = Dispatcher::new(id("unknown"));
        dispatcher
            .register_trigger(trigger(
                "low",
                1,
                TriggerFlow::Consume,
                Some(InputCommand::Key {
                    android_key_code: 4,
                }),
                Arc::clone(&calls),
            ))
            .unwrap();
        dispatcher
            .register_trigger(trigger(
                "high",
                100,
                TriggerFlow::Consume,
                Some(InputCommand::Key {
                    android_key_code: 3,
                }),
                Arc::clone(&calls),
            ))
            .unwrap();

        let report = dispatcher.tick(&frame(), &[]).unwrap();

        assert_eq!(*calls.lock().unwrap(), ["high"]);
        assert_eq!(report.fired_triggers, [TriggerId::new("high").unwrap()]);
        assert_eq!(
            report.input.unwrap().command,
            InputCommand::Key {
                android_key_code: 3
            }
        );
    }

    #[test]
    fn applies_a_trigger_state_transition() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut transition = trigger("battle", 0, TriggerFlow::Consume, None, calls);
        transition.transition = Some(id("battle"));
        let mut dispatcher = Dispatcher::new(id("home"));
        dispatcher.register_trigger(transition).unwrap();

        let report = dispatcher.tick(&frame(), &[]).unwrap();

        assert_eq!(dispatcher.state().current(), &id("battle"));
        assert_eq!(dispatcher.state().previous(), Some(&id("home")));
        assert_eq!(dispatcher.state().revision(), 1);
        assert_eq!(report.transition.unwrap().to, id("battle"));
    }

    #[test]
    fn pauses_resumes_and_completes_a_task() {
        let updates = Arc::new(Mutex::new(0));
        let task_id = TaskId::new("daily").unwrap();
        let mut dispatcher = Dispatcher::new(id("home"));
        dispatcher
            .register_task(CompletingTask {
                id: task_id.clone(),
                updates: Arc::clone(&updates),
            })
            .unwrap();

        let started = dispatcher.start_task(&task_id, &frame(), &[]).unwrap();
        assert_eq!(
            started.task_event,
            Some(TaskEvent::Started(task_id.clone()))
        );
        dispatcher.pause_task().unwrap();
        dispatcher.tick(&frame(), &[]).unwrap();
        assert_eq!(*updates.lock().unwrap(), 0);
        assert_eq!(
            dispatcher.active_task_state(),
            Some(ActiveTaskState::Paused)
        );

        dispatcher.resume_task().unwrap();
        let completed = dispatcher.tick(&frame(), &[]).unwrap();

        assert_eq!(*updates.lock().unwrap(), 1);
        assert_eq!(completed.task_event, Some(TaskEvent::Completed(task_id)));
        assert!(completed.input.is_some());
        assert!(dispatcher.active_task_id().is_none());
    }

    #[test]
    fn stops_an_active_task_before_the_next_tick() {
        let updates = Arc::new(Mutex::new(0));
        let task_id = TaskId::new("daily").unwrap();
        let mut dispatcher = Dispatcher::new(id("home"));
        dispatcher
            .register_task(CompletingTask {
                id: task_id.clone(),
                updates: Arc::clone(&updates),
            })
            .unwrap();
        dispatcher.start_task(&task_id, &frame(), &[]).unwrap();

        let stopped = dispatcher.stop_task(&frame(), &[]).unwrap();
        dispatcher.tick(&frame(), &[]).unwrap();

        assert_eq!(stopped.task_event, Some(TaskEvent::Stopped(task_id)));
        assert!(dispatcher.active_task_id().is_none());
        assert_eq!(*updates.lock().unwrap(), 0);
    }

    #[test]
    fn consuming_trigger_suspends_the_active_task_for_the_tick() {
        let updates = Arc::new(Mutex::new(0));
        let task_id = TaskId::new("daily").unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut dispatcher = Dispatcher::new(id("home"));
        dispatcher
            .register_task(CompletingTask {
                id: task_id.clone(),
                updates: Arc::clone(&updates),
            })
            .unwrap();
        dispatcher
            .register_trigger(trigger(
                "recovery",
                1_000,
                TriggerFlow::Consume,
                None,
                calls,
            ))
            .unwrap();
        dispatcher.start_task(&task_id, &frame(), &[]).unwrap();

        dispatcher.tick(&frame(), &[]).unwrap();

        assert_eq!(*updates.lock().unwrap(), 0);
        assert_eq!(dispatcher.active_task_id(), Some(&task_id));
    }

    #[test]
    fn rejects_more_than_one_input_in_a_tick() {
        let updates = Arc::new(Mutex::new(0));
        let task_id = TaskId::new("daily").unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut dispatcher = Dispatcher::new(id("home"));
        dispatcher
            .register_task(CompletingTask {
                id: task_id.clone(),
                updates,
            })
            .unwrap();
        dispatcher
            .register_trigger(trigger(
                "dialog",
                10,
                TriggerFlow::Continue,
                Some(InputCommand::Key {
                    android_key_code: 3,
                }),
                calls,
            ))
            .unwrap();
        dispatcher.start_task(&task_id, &frame(), &[]).unwrap();

        assert_eq!(
            dispatcher.tick(&frame(), &[]).unwrap_err(),
            DispatcherError::MultipleInputsInTick
        );
    }
}
