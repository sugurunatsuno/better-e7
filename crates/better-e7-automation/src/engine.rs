use std::{collections::BTreeMap, error::Error, fmt, time::Duration};

use better_e7_core::{Detection, InputCommand, NormalizedPoint};

use crate::{Action, AutomationProfile, AutomationRule, Condition, ProfileError};

pub struct AutomationEngine {
    profile: AutomationProfile,
    evaluation_order: Vec<usize>,
    last_fired: BTreeMap<String, Duration>,
}

impl AutomationEngine {
    pub fn new(profile: AutomationProfile) -> Result<Self, ProfileError> {
        profile.validate()?;
        let mut evaluation_order = (0..profile.rules.len()).collect::<Vec<_>>();
        evaluation_order.sort_by(|left, right| {
            profile.rules[*right]
                .priority
                .cmp(&profile.rules[*left].priority)
                .then(left.cmp(right))
        });
        Ok(Self {
            profile,
            evaluation_order,
            last_fired: BTreeMap::new(),
        })
    }

    #[must_use]
    pub const fn profile(&self) -> &AutomationProfile {
        &self.profile
    }

    pub fn reset(&mut self) {
        self.last_fired.clear();
    }

    pub fn tick(
        &mut self,
        detections: &[Detection],
        elapsed: Duration,
    ) -> Result<AutomationReport, EngineError> {
        let mut report = AutomationReport::default();
        for &index in &self.evaluation_order {
            let rule = &self.profile.rules[index];
            if !rule.enabled
                || self.is_cooling_down(rule, elapsed)
                || !matches_condition(&rule.condition, detections)
            {
                continue;
            }

            let effect = resolve_action(rule, detections)?;
            self.last_fired.insert(rule.id.clone(), elapsed);
            report.fired_rules.push(rule.id.clone());
            match effect {
                ResolvedEffect::Input(command) => {
                    report.input = Some(AutomationInput {
                        rule_id: rule.id.clone(),
                        command,
                    });
                    break;
                }
                ResolvedEffect::Log(message) => report.logs.push(message),
            }
            if rule.consume {
                break;
            }
        }
        Ok(report)
    }

    fn is_cooling_down(&self, rule: &AutomationRule, elapsed: Duration) -> bool {
        let Some(last_fired) = self.last_fired.get(&rule.id) else {
            return false;
        };
        elapsed.saturating_sub(*last_fired) < Duration::from_millis(rule.cooldown_ms)
    }
}

fn matches_condition(condition: &Condition, detections: &[Detection]) -> bool {
    match condition {
        Condition::Always => true,
        Condition::DetectionPresent {
            label,
            minimum_confidence,
        } => best_detection(detections, label, *minimum_confidence).is_some(),
        Condition::DetectionAbsent {
            label,
            minimum_confidence,
        } => best_detection(detections, label, *minimum_confidence).is_none(),
        Condition::All { conditions } => conditions
            .iter()
            .all(|condition| matches_condition(condition, detections)),
        Condition::Any { conditions } => conditions
            .iter()
            .any(|condition| matches_condition(condition, detections)),
        Condition::Not { condition } => !matches_condition(condition, detections),
    }
}

fn resolve_action(
    rule: &AutomationRule,
    detections: &[Detection],
) -> Result<ResolvedEffect, EngineError> {
    let effect = match &rule.action {
        Action::TapDetection { label } => {
            let detection = best_detection(detections, label, 0.0).ok_or_else(|| {
                EngineError::TargetNotDetected {
                    rule_id: rule.id.clone(),
                    label: label.clone(),
                }
            })?;
            ResolvedEffect::Input(InputCommand::Tap {
                point: detection.center,
            })
        }
        Action::Tap { x, y } => ResolvedEffect::Input(InputCommand::Tap {
            point: normalized_point(*x, *y, &rule.id)?,
        }),
        Action::Swipe {
            from_x,
            from_y,
            to_x,
            to_y,
            duration_ms,
        } => ResolvedEffect::Input(InputCommand::Swipe {
            from: normalized_point(*from_x, *from_y, &rule.id)?,
            to: normalized_point(*to_x, *to_y, &rule.id)?,
            duration: Duration::from_millis(*duration_ms),
        }),
        Action::Key { android_key_code } => ResolvedEffect::Input(InputCommand::Key {
            android_key_code: *android_key_code,
        }),
        Action::Log { message } => ResolvedEffect::Log(message.clone()),
    };
    Ok(effect)
}

fn normalized_point(x: f32, y: f32, rule_id: &str) -> Result<NormalizedPoint, EngineError> {
    NormalizedPoint::new(x, y).map_err(|error| EngineError::InvalidAction {
        rule_id: rule_id.to_owned(),
        message: error.to_string(),
    })
}

fn best_detection<'a>(
    detections: &'a [Detection],
    label: &str,
    minimum_confidence: f32,
) -> Option<&'a Detection> {
    detections
        .iter()
        .filter(|detection| detection.label == label && detection.confidence >= minimum_confidence)
        .max_by(|left, right| left.confidence.total_cmp(&right.confidence))
}

enum ResolvedEffect {
    Input(InputCommand),
    Log(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AutomationInput {
    pub rule_id: String,
    pub command: InputCommand,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AutomationReport {
    pub input: Option<AutomationInput>,
    pub fired_rules: Vec<String>,
    pub logs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    TargetNotDetected { rule_id: String, label: String },
    InvalidAction { rule_id: String, message: String },
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetNotDetected { rule_id, label } => {
                write!(formatter, "rule target is not detected: {rule_id}: {label}")
            }
            Self::InvalidAction { rule_id, message } => {
                write!(formatter, "rule action is invalid: {rule_id}: {message}")
            }
        }
    }
}

impl Error for EngineError {}

#[cfg(test)]
mod tests {
    use better_e7_core::NormalizedRect;

    use super::*;

    #[test]
    fn taps_the_best_matching_detection() {
        let mut engine = AutomationEngine::new(profile(vec![rule(
            "confirm",
            100,
            1_000,
            true,
            Condition::DetectionPresent {
                label: "confirm".to_owned(),
                minimum_confidence: 0.9,
            },
            Action::TapDetection {
                label: "confirm".to_owned(),
            },
        )]))
        .unwrap();
        let detections = [
            detection("confirm", 0.91, 0.2),
            detection("confirm", 0.98, 0.8),
        ];

        let report = engine.tick(&detections, Duration::ZERO).unwrap();

        assert_eq!(report.fired_rules, ["confirm"]);
        assert_eq!(
            report.input.unwrap().command,
            InputCommand::Tap {
                point: NormalizedPoint::new(0.8, 0.8).unwrap()
            }
        );
    }

    #[test]
    fn uses_priority_and_cooldown_deterministically() {
        let mut engine = AutomationEngine::new(profile(vec![
            rule(
                "low",
                10,
                0,
                true,
                Condition::Always,
                Action::Key {
                    android_key_code: 4,
                },
            ),
            rule(
                "high",
                100,
                1_000,
                true,
                Condition::Always,
                Action::Key {
                    android_key_code: 3,
                },
            ),
        ]))
        .unwrap();

        let first = engine.tick(&[], Duration::ZERO).unwrap();
        let while_cooling = engine.tick(&[], Duration::from_millis(500)).unwrap();
        let after_cooldown = engine.tick(&[], Duration::from_millis(1_000)).unwrap();

        assert_eq!(first.input.unwrap().rule_id, "high");
        assert_eq!(while_cooling.input.unwrap().rule_id, "low");
        assert_eq!(after_cooldown.input.unwrap().rule_id, "high");
    }

    #[test]
    fn consuming_log_rule_stops_lower_priority_rules() {
        let mut engine = AutomationEngine::new(profile(vec![
            rule(
                "observe",
                100,
                0,
                true,
                Condition::Always,
                Action::Log {
                    message: "observed".to_owned(),
                },
            ),
            rule(
                "input",
                10,
                0,
                true,
                Condition::Always,
                Action::Key {
                    android_key_code: 3,
                },
            ),
        ]))
        .unwrap();

        let report = engine.tick(&[], Duration::ZERO).unwrap();

        assert_eq!(report.fired_rules, ["observe"]);
        assert_eq!(report.logs, ["observed"]);
        assert!(report.input.is_none());
    }

    #[test]
    fn non_consuming_log_rule_allows_one_input() {
        let observe = rule(
            "observe",
            100,
            0,
            false,
            Condition::Always,
            Action::Log {
                message: "observed".to_owned(),
            },
        );
        let mut engine = AutomationEngine::new(profile(vec![
            observe,
            rule(
                "input",
                10,
                0,
                true,
                Condition::Always,
                Action::Key {
                    android_key_code: 3,
                },
            ),
        ]))
        .unwrap();

        let report = engine.tick(&[], Duration::ZERO).unwrap();

        assert_eq!(report.fired_rules, ["observe", "input"]);
        assert_eq!(report.logs, ["observed"]);
        assert_eq!(report.input.unwrap().rule_id, "input");
    }

    #[test]
    fn evaluates_nested_conditions() {
        let condition = Condition::All {
            conditions: vec![
                Condition::DetectionPresent {
                    label: "ready".to_owned(),
                    minimum_confidence: 0.9,
                },
                Condition::Not {
                    condition: Box::new(Condition::DetectionPresent {
                        label: "blocked".to_owned(),
                        minimum_confidence: 0.9,
                    }),
                },
            ],
        };
        let mut engine = AutomationEngine::new(profile(vec![rule(
            "nested",
            0,
            0,
            true,
            condition,
            Action::Key {
                android_key_code: 3,
            },
        )]))
        .unwrap();

        assert!(
            engine
                .tick(&[detection("ready", 0.95, 0.5)], Duration::ZERO)
                .unwrap()
                .input
                .is_some()
        );
        engine.reset();
        assert!(
            engine
                .tick(
                    &[
                        detection("ready", 0.95, 0.5),
                        detection("blocked", 0.95, 0.5),
                    ],
                    Duration::ZERO,
                )
                .unwrap()
                .input
                .is_none()
        );
    }

    #[test]
    fn reports_a_missing_tap_target() {
        let mut engine = AutomationEngine::new(profile(vec![rule(
            "broken",
            0,
            0,
            true,
            Condition::Always,
            Action::TapDetection {
                label: "missing".to_owned(),
            },
        )]))
        .unwrap();

        assert_eq!(
            engine.tick(&[], Duration::ZERO).unwrap_err(),
            EngineError::TargetNotDetected {
                rule_id: "broken".to_owned(),
                label: "missing".to_owned(),
            }
        );
    }

    fn profile(rules: Vec<AutomationRule>) -> AutomationProfile {
        AutomationProfile {
            name: "test".to_owned(),
            templates: Vec::new(),
            rules,
        }
    }

    fn rule(
        id: &str,
        priority: i32,
        cooldown_ms: u64,
        consume: bool,
        condition: Condition,
        action: Action,
    ) -> AutomationRule {
        AutomationRule {
            id: id.to_owned(),
            enabled: true,
            priority,
            cooldown_ms,
            consume,
            condition,
            action,
        }
    }

    fn detection(label: &str, confidence: f32, center: f32) -> Detection {
        let half_size = 0.05;
        Detection {
            label: label.to_owned(),
            confidence,
            center: NormalizedPoint::new(center, center).unwrap(),
            bounds: NormalizedRect::new(
                center - half_size,
                center - half_size,
                center + half_size,
                center + half_size,
            )
            .unwrap(),
        }
    }
}
