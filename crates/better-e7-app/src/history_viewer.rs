use std::path::PathBuf;

use better_e7_runtime::AutomationHistoryRecord;
use eframe::egui;

pub struct HistoryViewer {
    open: bool,
    path: Option<PathBuf>,
    records: Vec<AutomationHistoryRecord>,
    profile_filter: String,
    rule_filter: String,
    event_filter: String,
}

impl HistoryViewer {
    pub fn new() -> Self {
        Self {
            open: false,
            path: None,
            records: Vec::new(),
            profile_filter: String::new(),
            rule_filter: String::new(),
            event_filter: String::new(),
        }
    }

    pub fn loaded(&mut self, path: PathBuf, records: Vec<AutomationHistoryRecord>) {
        self.path = Some(path);
        self.records = records;
        self.open = true;
    }

    pub fn show(&mut self, context: &egui::Context) -> Option<PathBuf> {
        if !self.open {
            return None;
        }
        let mut open = self.open;
        let mut reload = None;
        egui::Window::new("実行履歴")
            .open(&mut open)
            .default_width(900.0)
            .default_height(560.0)
            .resizable(true)
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        self.path
                            .as_ref()
                            .map(|path| path.to_string_lossy())
                            .as_deref()
                            .unwrap_or("未読み込み"),
                    );
                    if ui.button("再読み込み").clicked()
                        && let Some(path) = self.path.clone()
                    {
                        reload = Some(path);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("profile");
                    ui.text_edit_singleline(&mut self.profile_filter);
                    ui.label("Rule");
                    ui.text_edit_singleline(&mut self.rule_filter);
                    ui.label("event");
                    ui.text_edit_singleline(&mut self.event_filter);
                });
                let visible = self
                    .records
                    .iter()
                    .filter(|record| self.matches(record))
                    .collect::<Vec<_>>();
                ui.label(format!(
                    "{} / {} records",
                    visible.len(),
                    self.records.len()
                ));
                ui.separator();
                egui::ScrollArea::both().show(ui, |ui| {
                    egui::Grid::new("history-grid")
                        .striped(true)
                        .min_col_width(90.0)
                        .show(ui, |ui| {
                            ui.label("elapsed ms");
                            ui.label("profile");
                            ui.label("Rule");
                            ui.label("event");
                            ui.label("detail");
                            ui.end_row();
                            for record in visible {
                                ui.monospace(
                                    record
                                        .session_elapsed_ms
                                        .map(|value| value.to_string())
                                        .unwrap_or_else(|| "--".to_owned()),
                                );
                                ui.label(record.profile.as_deref().unwrap_or("--"));
                                ui.label(&record.rule_id);
                                ui.monospace(record.event.as_str());
                                ui.monospace(record.detail.as_deref().unwrap_or("--"));
                                ui.end_row();
                            }
                        });
                });
            });
        self.open = open;
        reload
    }

    fn matches(&self, record: &AutomationHistoryRecord) -> bool {
        contains(
            record.profile.as_deref().unwrap_or_default(),
            &self.profile_filter,
        ) && contains(&record.rule_id, &self.rule_filter)
            && contains(record.event.as_str(), &self.event_filter)
    }
}

fn contains(value: &str, filter: &str) -> bool {
    filter.is_empty() || value.contains(filter)
}
