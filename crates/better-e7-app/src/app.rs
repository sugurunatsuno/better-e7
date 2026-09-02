use std::time::Duration;

use better_e7_adb::AdbDevice;
use better_e7_config::AppConfig;
use better_e7_runtime::{AppRuntime, AutomationState, RuntimeCommand, RuntimeEvent};
use eframe::egui;
use tracing::{error, info};

const MAX_VISIBLE_LOGS: usize = 500;

pub struct BetterE7App {
    runtime: Option<AppRuntime>,
    devices: Vec<AdbDevice>,
    selected_device: Option<String>,
    automation_state: AutomationState,
    logs: Vec<String>,
}

impl BetterE7App {
    pub fn new(_creation_context: &eframe::CreationContext<'_>, config: AppConfig) -> Self {
        let mut app = Self {
            runtime: None,
            devices: Vec::new(),
            selected_device: None,
            automation_state: AutomationState::Stopped,
            logs: vec!["better-e7を起動しました".to_owned()],
        };

        match AppRuntime::new(&config) {
            Ok(runtime) => {
                app.runtime = Some(runtime);
                app.push_log("ADB端末の監視を開始しました");
            }
            Err(error) => {
                error!(%error, "runtime initialization failed");
                app.push_log(format!("Runtimeの初期化に失敗しました: {error}"));
            }
        }
        app
    }

    fn drain_runtime_events(&mut self) {
        let events = self
            .runtime
            .as_mut()
            .map(|runtime| std::iter::from_fn(|| runtime.try_next_event()).collect::<Vec<_>>())
            .unwrap_or_default();

        for event in events {
            match event {
                RuntimeEvent::DevicesUpdated(devices) => {
                    if self.devices != devices {
                        let count = devices.len();
                        self.devices = devices;
                        self.push_log(format!("ADB端末を{count}台検出しました"));
                    }
                }
                RuntimeEvent::SelectedDeviceChanged(serial) => {
                    self.selected_device = serial;
                    if let Some(serial) = &self.selected_device {
                        self.push_log(format!("端末を選択しました: {serial}"));
                    } else {
                        self.push_log("選択中の端末が切断されました");
                    }
                }
                RuntimeEvent::AutomationStateChanged(state) => {
                    self.automation_state = state;
                    self.push_log(match state {
                        AutomationState::Stopped => "自動化を停止しました",
                        AutomationState::Running => "自動化を開始しました",
                    });
                }
                RuntimeEvent::Error(message) => {
                    error!(%message, "runtime error");
                    self.push_log(format!("エラー: {message}"));
                }
            }
        }
    }

    fn send(&mut self, command: RuntimeCommand) {
        let result = self
            .runtime
            .as_ref()
            .ok_or_else(|| "Runtimeが利用できません".to_owned())
            .and_then(|runtime| runtime.send(command).map_err(|error| error.to_string()));
        if let Err(message) = result {
            self.push_log(format!("エラー: {message}"));
        }
    }

    fn push_log(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self.logs.last() == Some(&message) {
            return;
        }
        info!(%message);
        self.logs.push(message);
        if self.logs.len() > MAX_VISIBLE_LOGS {
            self.logs.remove(0);
        }
    }

    fn show_toolbar(&mut self, context: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").show(context, |ui| {
            ui.horizontal(|ui| {
                ui.heading("better-e7");
                ui.separator();
                ui.label(match self.automation_state {
                    AutomationState::Stopped => "停止中",
                    AutomationState::Running => "実行中",
                });

                let can_toggle = self.runtime.is_some()
                    && (self.automation_state == AutomationState::Running
                        || self.selected_device.is_some());
                let label = match self.automation_state {
                    AutomationState::Stopped => "開始",
                    AutomationState::Running => "停止",
                };
                if ui
                    .add_enabled(can_toggle, egui::Button::new(label))
                    .clicked()
                {
                    let command = match self.automation_state {
                        AutomationState::Stopped => RuntimeCommand::StartAutomation,
                        AutomationState::Running => RuntimeCommand::StopAutomation,
                    };
                    self.send(command);
                }
            });
        });
    }

    fn show_devices(&mut self, context: &egui::Context) {
        egui::SidePanel::left("devices")
            .resizable(true)
            .default_width(260.0)
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("端末");
                    if ui
                        .add_enabled(self.runtime.is_some(), egui::Button::new("再読み込み"))
                        .clicked()
                    {
                        self.send(RuntimeCommand::RefreshDevices);
                    }
                });

                if self.devices.is_empty() {
                    ui.label("ADB端末が見つかりません");
                }

                let mut selected = None;
                for device in &self.devices {
                    let is_selected = self.selected_device.as_deref() == Some(&device.serial);
                    let label = format!(
                        "{}\n{} / {}",
                        device.display_name(),
                        device.serial,
                        device.state
                    );
                    if ui
                        .add_enabled(
                            device.is_ready() && self.automation_state == AutomationState::Stopped,
                            egui::Button::selectable(is_selected, label),
                        )
                        .clicked()
                    {
                        selected = Some(device.serial.clone());
                    }
                }
                if let Some(serial) = selected {
                    self.send(RuntimeCommand::SelectDevice(serial));
                }

                ui.separator();
                ui.heading("タスク");
                ui.label("ゲームプラグインは未実装です");
            });
    }

    fn show_preview(&self, context: &egui::Context) {
        egui::CentralPanel::default().show(context, |ui| {
            ui.heading("プレビュー");
            let available = ui.available_size();
            let preview_height = (available.y - 120.0).max(160.0);
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(available.x, preview_height),
                egui::Sense::hover(),
            );
            ui.painter()
                .rect_filled(rect, 6.0, egui::Color32::from_gray(24));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Android映像は未接続です",
                egui::FontId::proportional(22.0),
                egui::Color32::GRAY,
            );
            ui.separator();
            ui.heading("状態");
            ui.label(format!(
                "端末: {}",
                self.selected_device.as_deref().unwrap_or("未選択")
            ));
            ui.label("認識FPS: --");
            ui.label("ゲーム状態: Unknown");
        });
    }

    fn show_logs(&self, context: &egui::Context) {
        egui::TopBottomPanel::bottom("logs")
            .resizable(true)
            .default_height(140.0)
            .show(context, |ui| {
                ui.heading("ログ");
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for message in &self.logs {
                            ui.monospace(message);
                        }
                    });
            });
    }
}

impl eframe::App for BetterE7App {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_runtime_events();
        self.show_toolbar(context);
        self.show_devices(context);
        self.show_preview(context);
        self.show_logs(context);
        context.request_repaint_after(Duration::from_millis(100));
    }
}
