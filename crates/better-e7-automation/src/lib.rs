mod engine;
mod profile;

pub use engine::{AutomationEngine, AutomationInput, AutomationReport, EngineError};
pub use profile::{
    Action, AutomationProfile, AutomationRule, Condition, ProfileError, TemplateDefinition,
    TemplateRegion,
};
