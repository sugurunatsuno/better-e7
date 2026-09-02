mod dispatcher;
mod id;
mod model;
mod plugin;

pub use dispatcher::{ComponentError, Dispatcher, DispatcherError, Task, Trigger};
pub use id::{GameId, IdError, StateId, TaskId, TriggerId};
pub use model::{
    ActiveTaskState, DispatchPlan, DispatchReport, GameContext, GameState, InputIntent, PlanError,
    StateTransition, TaskEvent, TaskStep, TriggerFlow, TriggerOutcome,
};
pub use plugin::{
    DescriptorError, GameDescriptor, GamePlugin, GameRegistry, PluginError, RegistryError,
};
