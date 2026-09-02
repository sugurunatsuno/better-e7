use std::path::PathBuf;

use better_e7_automation::{
    Action, AutomationProfile, AutomationRule, Condition, TemplateDefinition, TemplateRegion,
};
use eframe::egui;

pub enum ProfileEditorCommand {
    Load(PathBuf),
    Save {
        path: PathBuf,
        profile: AutomationProfile,
    },
}

pub struct ProfileEditor {
    open: bool,
    path: String,
    profile: Option<AutomationProfile>,
    status: String,
}

impl ProfileEditor {
    pub fn new(path: String) -> Self {
        Self {
            open: false,
            path,
            profile: None,
            status: "profileを読み込むか、新規作成してください".to_owned(),
        }
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn set_path(&mut self, path: &str) {
        self.path = path.to_owned();
    }

    pub fn loaded(&mut self, path: PathBuf, profile: AutomationProfile) {
        self.path = path.to_string_lossy().into_owned();
        self.status = format!("{}を読み込みました", profile.name);
        self.profile = Some(profile);
        self.open = true;
    }

    pub fn saved(&mut self, path: PathBuf, name: &str) {
        self.path = path.to_string_lossy().into_owned();
        self.status = format!("{name}を保存しました");
    }

    pub fn show(&mut self, context: &egui::Context, enabled: bool) -> Option<ProfileEditorCommand> {
        if !self.open {
            return None;
        }
        let mut open = self.open;
        let mut command = None;
        egui::Window::new("Rule editor")
            .open(&mut open)
            .default_width(720.0)
            .default_height(620.0)
            .resizable(true)
            .show(context, |ui| {
                ui.add_enabled(
                    enabled,
                    egui::TextEdit::singleline(&mut self.path).hint_text("automation.toml"),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            enabled && !self.path.trim().is_empty(),
                            egui::Button::new("読み込み"),
                        )
                        .clicked()
                    {
                        command = Some(ProfileEditorCommand::Load(PathBuf::from(self.path.trim())));
                    }
                    if ui.add_enabled(enabled, egui::Button::new("新規")).clicked() {
                        self.profile = Some(empty_profile());
                        self.status = "新しいprofileを編集中です".to_owned();
                    }
                    if ui
                        .add_enabled(
                            enabled && self.profile.is_some() && !self.path.trim().is_empty(),
                            egui::Button::new("検証して保存"),
                        )
                        .clicked()
                        && let Some(profile) = self.profile.clone()
                    {
                        command = Some(ProfileEditorCommand::Save {
                            path: PathBuf::from(self.path.trim()),
                            profile,
                        });
                    }
                });
                ui.label(&self.status);
                ui.separator();
                if let Some(profile) = self.profile.as_mut() {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        show_profile(ui, profile, enabled);
                    });
                }
            });
        self.open = open;
        command
    }
}

fn empty_profile() -> AutomationProfile {
    AutomationProfile {
        name: "new-profile".to_owned(),
        templates: Vec::new(),
        rules: Vec::new(),
    }
}

fn show_profile(ui: &mut egui::Ui, profile: &mut AutomationProfile, enabled: bool) {
    ui.add_enabled(
        enabled,
        egui::TextEdit::singleline(&mut profile.name).hint_text("profile name"),
    );
    ui.separator();
    ui.heading("Templates");
    let mut removed_template = None;
    for (index, template) in profile.templates.iter_mut().enumerate() {
        ui.push_id(("template", index), |ui| {
            egui::CollapsingHeader::new(format!("{} / {}", index + 1, template.id))
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("ID");
                        ui.add_enabled(enabled, egui::TextEdit::singleline(&mut template.id));
                    });
                    ui.horizontal(|ui| {
                        ui.label("画像");
                        let mut path = template.path.to_string_lossy().into_owned();
                        if ui
                            .add_enabled(enabled, egui::TextEdit::singleline(&mut path))
                            .changed()
                        {
                            template.path = PathBuf::from(path);
                        }
                    });
                    ui.add_enabled(
                        enabled,
                        egui::Slider::new(&mut template.threshold, 0.0..=1.0).text("threshold"),
                    );
                    show_region(ui, &mut template.region, enabled);
                    if ui
                        .add_enabled(enabled, egui::Button::new("Templateを削除"))
                        .clicked()
                    {
                        removed_template = Some(index);
                    }
                });
        });
    }
    if let Some(index) = removed_template {
        profile.templates.remove(index);
    }
    if ui
        .add_enabled(enabled, egui::Button::new("Templateを追加"))
        .clicked()
    {
        profile.templates.push(TemplateDefinition {
            id: next_id("template", profile.templates.len()),
            path: PathBuf::new(),
            threshold: 0.9,
            region: TemplateRegion::default(),
        });
    }

    ui.separator();
    ui.heading("Rules");
    let template_ids = profile
        .templates
        .iter()
        .map(|template| template.id.clone())
        .collect::<Vec<_>>();
    let mut removed_rule = None;
    for (index, rule) in profile.rules.iter_mut().enumerate() {
        ui.push_id(("rule", index), |ui| {
            egui::CollapsingHeader::new(format!("{} / {}", index + 1, rule.id))
                .default_open(true)
                .show(ui, |ui| {
                    show_rule(ui, rule, &template_ids, enabled);
                    if ui
                        .add_enabled(enabled, egui::Button::new("Ruleを削除"))
                        .clicked()
                    {
                        removed_rule = Some(index);
                    }
                });
        });
    }
    if let Some(index) = removed_rule {
        profile.rules.remove(index);
    }
    if ui
        .add_enabled(enabled, egui::Button::new("Ruleを追加"))
        .clicked()
    {
        profile.rules.push(AutomationRule {
            id: next_id("rule", profile.rules.len()),
            enabled: true,
            priority: 0,
            cooldown_ms: 0,
            consume: true,
            condition: Condition::Always,
            action: Action::Log {
                message: "matched".to_owned(),
            },
        });
    }
}

fn show_region(ui: &mut egui::Ui, region: &mut TemplateRegion, enabled: bool) {
    ui.horizontal(|ui| {
        ui.label("ROI");
        coordinate(ui, "left", &mut region.left, enabled);
        coordinate(ui, "top", &mut region.top, enabled);
        coordinate(ui, "right", &mut region.right, enabled);
        coordinate(ui, "bottom", &mut region.bottom, enabled);
    });
}

fn coordinate(ui: &mut egui::Ui, label: &str, value: &mut f32, enabled: bool) {
    ui.label(label);
    ui.add_enabled(
        enabled,
        egui::DragValue::new(value).speed(0.01).range(0.0..=1.0),
    );
}

fn show_rule(ui: &mut egui::Ui, rule: &mut AutomationRule, template_ids: &[String], enabled: bool) {
    ui.horizontal(|ui| {
        ui.label("ID");
        ui.add_enabled(enabled, egui::TextEdit::singleline(&mut rule.id));
        ui.add_enabled(enabled, egui::Checkbox::new(&mut rule.enabled, "enabled"));
        ui.add_enabled(enabled, egui::Checkbox::new(&mut rule.consume, "consume"));
    });
    ui.horizontal(|ui| {
        ui.label("priority");
        ui.add_enabled(enabled, egui::DragValue::new(&mut rule.priority));
        ui.label("cooldown ms");
        ui.add_enabled(enabled, egui::DragValue::new(&mut rule.cooldown_ms));
    });
    ui.group(|ui| {
        ui.label("Condition");
        show_condition(ui, &mut rule.condition, template_ids, enabled);
    });
    ui.group(|ui| {
        ui.label("Action");
        show_action(ui, &mut rule.action, template_ids, enabled);
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConditionKind {
    Always,
    Present,
    Absent,
    All,
    Any,
    Not,
}

fn show_condition(
    ui: &mut egui::Ui,
    condition: &mut Condition,
    template_ids: &[String],
    enabled: bool,
) {
    let current = condition_kind(condition);
    let mut selected = current;
    ui.add_enabled_ui(enabled, |ui| {
        egui::ComboBox::from_id_salt("condition-kind")
            .selected_text(condition_kind_name(selected))
            .show_ui(ui, |ui| {
                for kind in [
                    ConditionKind::Always,
                    ConditionKind::Present,
                    ConditionKind::Absent,
                    ConditionKind::All,
                    ConditionKind::Any,
                    ConditionKind::Not,
                ] {
                    ui.selectable_value(&mut selected, kind, condition_kind_name(kind));
                }
            });
    });
    if selected != current {
        *condition = default_condition(selected, template_ids);
    }
    match condition {
        Condition::Always => {}
        Condition::DetectionPresent {
            label,
            minimum_confidence,
        }
        | Condition::DetectionAbsent {
            label,
            minimum_confidence,
        } => {
            show_label(ui, label, template_ids, enabled);
            ui.add_enabled(
                enabled,
                egui::Slider::new(minimum_confidence, 0.0..=1.0).text("minimum confidence"),
            );
        }
        Condition::All { conditions } | Condition::Any { conditions } => {
            let mut removed = None;
            for (index, nested) in conditions.iter_mut().enumerate() {
                ui.push_id(("condition", index), |ui| {
                    ui.group(|ui| {
                        show_condition(ui, nested, template_ids, enabled);
                        if ui
                            .add_enabled(enabled, egui::Button::new("条件を削除"))
                            .clicked()
                        {
                            removed = Some(index);
                        }
                    });
                });
            }
            if let Some(index) = removed {
                conditions.remove(index);
            }
            if ui
                .add_enabled(enabled, egui::Button::new("条件を追加"))
                .clicked()
            {
                conditions.push(Condition::Always);
            }
        }
        Condition::Not { condition } => {
            ui.push_id("not-condition", |ui| {
                show_condition(ui, condition, template_ids, enabled);
            });
        }
    }
}

fn condition_kind(condition: &Condition) -> ConditionKind {
    match condition {
        Condition::Always => ConditionKind::Always,
        Condition::DetectionPresent { .. } => ConditionKind::Present,
        Condition::DetectionAbsent { .. } => ConditionKind::Absent,
        Condition::All { .. } => ConditionKind::All,
        Condition::Any { .. } => ConditionKind::Any,
        Condition::Not { .. } => ConditionKind::Not,
    }
}

const fn condition_kind_name(kind: ConditionKind) -> &'static str {
    match kind {
        ConditionKind::Always => "always",
        ConditionKind::Present => "detection present",
        ConditionKind::Absent => "detection absent",
        ConditionKind::All => "all",
        ConditionKind::Any => "any",
        ConditionKind::Not => "not",
    }
}

fn default_condition(kind: ConditionKind, template_ids: &[String]) -> Condition {
    let label = default_label(template_ids);
    match kind {
        ConditionKind::Always => Condition::Always,
        ConditionKind::Present => Condition::DetectionPresent {
            label,
            minimum_confidence: 0.9,
        },
        ConditionKind::Absent => Condition::DetectionAbsent {
            label,
            minimum_confidence: 0.9,
        },
        ConditionKind::All => Condition::All {
            conditions: vec![Condition::Always],
        },
        ConditionKind::Any => Condition::Any {
            conditions: vec![Condition::Always],
        },
        ConditionKind::Not => Condition::Not {
            condition: Box::new(Condition::Always),
        },
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActionKind {
    TapDetection,
    Tap,
    Swipe,
    Key,
    Log,
}

fn show_action(ui: &mut egui::Ui, action: &mut Action, template_ids: &[String], enabled: bool) {
    let current = action_kind(action);
    let mut selected = current;
    ui.add_enabled_ui(enabled, |ui| {
        egui::ComboBox::from_id_salt("action-kind")
            .selected_text(action_kind_name(selected))
            .show_ui(ui, |ui| {
                for kind in [
                    ActionKind::TapDetection,
                    ActionKind::Tap,
                    ActionKind::Swipe,
                    ActionKind::Key,
                    ActionKind::Log,
                ] {
                    ui.selectable_value(&mut selected, kind, action_kind_name(kind));
                }
            });
    });
    if selected != current {
        *action = default_action(selected, template_ids);
    }
    match action {
        Action::TapDetection { label } => show_label(ui, label, template_ids, enabled),
        Action::Tap { x, y } => {
            ui.horizontal(|ui| {
                coordinate(ui, "x", x, enabled);
                coordinate(ui, "y", y, enabled);
            });
        }
        Action::Swipe {
            from_x,
            from_y,
            to_x,
            to_y,
            duration_ms,
        } => {
            ui.horizontal(|ui| {
                coordinate(ui, "from x", from_x, enabled);
                coordinate(ui, "from y", from_y, enabled);
            });
            ui.horizontal(|ui| {
                coordinate(ui, "to x", to_x, enabled);
                coordinate(ui, "to y", to_y, enabled);
            });
            ui.horizontal(|ui| {
                ui.label("duration ms");
                ui.add_enabled(enabled, egui::DragValue::new(duration_ms).range(1..=60_000));
            });
        }
        Action::Key { android_key_code } => {
            ui.horizontal(|ui| {
                ui.label("Android key code");
                ui.add_enabled(enabled, egui::DragValue::new(android_key_code));
            });
        }
        Action::Log { message } => {
            ui.add_enabled(
                enabled,
                egui::TextEdit::singleline(message).hint_text("message"),
            );
        }
    }
}

fn action_kind(action: &Action) -> ActionKind {
    match action {
        Action::TapDetection { .. } => ActionKind::TapDetection,
        Action::Tap { .. } => ActionKind::Tap,
        Action::Swipe { .. } => ActionKind::Swipe,
        Action::Key { .. } => ActionKind::Key,
        Action::Log { .. } => ActionKind::Log,
    }
}

const fn action_kind_name(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::TapDetection => "tap detection",
        ActionKind::Tap => "tap",
        ActionKind::Swipe => "swipe",
        ActionKind::Key => "key",
        ActionKind::Log => "log",
    }
}

fn default_action(kind: ActionKind, template_ids: &[String]) -> Action {
    match kind {
        ActionKind::TapDetection => Action::TapDetection {
            label: default_label(template_ids),
        },
        ActionKind::Tap => Action::Tap { x: 0.5, y: 0.5 },
        ActionKind::Swipe => Action::Swipe {
            from_x: 0.5,
            from_y: 0.8,
            to_x: 0.5,
            to_y: 0.2,
            duration_ms: 300,
        },
        ActionKind::Key => Action::Key {
            android_key_code: 4,
        },
        ActionKind::Log => Action::Log {
            message: "matched".to_owned(),
        },
    }
}

fn show_label(ui: &mut egui::Ui, label: &mut String, template_ids: &[String], enabled: bool) {
    ui.horizontal(|ui| {
        ui.label("label");
        ui.add_enabled(enabled, egui::TextEdit::singleline(label));
        if !template_ids.is_empty() {
            ui.add_enabled_ui(enabled, |ui| {
                egui::ComboBox::from_id_salt("label-template")
                    .selected_text("templateから選択")
                    .show_ui(ui, |ui| {
                        for template_id in template_ids {
                            if ui.button(template_id).clicked() {
                                *label = template_id.clone();
                            }
                        }
                    });
            });
        }
    });
}

fn default_label(template_ids: &[String]) -> String {
    template_ids.first().cloned().unwrap_or_default()
}

fn next_id(prefix: &str, current_len: usize) -> String {
    format!("{prefix}-{}", current_len + 1)
}
