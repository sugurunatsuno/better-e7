use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use better_e7_adb::AdbDevice;
use better_e7_config::AppConfig;
use better_e7_core::{
    Detection, Frame, InputCommand, NormalizedPoint, PixelFormat, PixelInputCommand,
};
use better_e7_runtime::{
    AppRuntime, AutomationState, ConnectionState, RuntimeCommand, RuntimeEvent,
};
use eframe::egui;
use tracing::{error, info};

use crate::profile_editor::{ProfileEditor, ProfileEditorCommand};

const MAX_VISIBLE_LOGS: usize = 500;

pub struct BetterE7App {
    config: AppConfig,
    runtime: Option<AppRuntime>,
    devices: Vec<AdbDevice>,
    selected_device: Option<String>,
    automation_state: AutomationState,
    connection_state: ConnectionState,
    video_bytes_received: u64,
    preview_texture: Option<egui::TextureHandle>,
    preview_resolution: Option<[usize; 2]>,
    detections: Vec<Detection>,
    recognition_fps: f32,
    recognition_updates: u32,
    recognition_window_started: Instant,
    automation_profile_name: Option<String>,
    automation_profile_path: String,
    last_profile_validation: Option<String>,
    profile_editor: ProfileEditor,
    automation_dry_run: bool,
    offline_frames_directory: String,
    offline_automation_running: bool,
    last_automation_rule: Option<String>,
    last_planned_input: Option<String>,
    logs: Vec<String>,
}

impl BetterE7App {
    pub fn new(_creation_context: &eframe::CreationContext<'_>, config: AppConfig) -> Self {
        let automation_profile_path = config
            .automation_profile_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut app = Self {
            config: config.clone(),
            runtime: None,
            devices: Vec::new(),
            selected_device: None,
            automation_state: AutomationState::Stopped,
            connection_state: ConnectionState::Disconnected,
            video_bytes_received: 0,
            preview_texture: None,
            preview_resolution: None,
            detections: Vec::new(),
            recognition_fps: 0.0,
            recognition_updates: 0,
            recognition_window_started: Instant::now(),
            automation_profile_name: None,
            automation_profile_path: automation_profile_path.clone(),
            last_profile_validation: None,
            profile_editor: ProfileEditor::new(automation_profile_path),
            automation_dry_run: config.automation_dry_run,
            offline_frames_directory: String::new(),
            offline_automation_running: false,
            last_automation_rule: None,
            last_planned_input: None,
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

    fn drain_runtime_events(&mut self, context: &egui::Context) {
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
                RuntimeEvent::ConnectionStateChanged(state) => {
                    self.connection_state = state;
                    if state == ConnectionState::Connecting {
                        self.video_bytes_received = 0;
                        self.preview_texture = None;
                        self.preview_resolution = None;
                        self.detections.clear();
                        self.recognition_fps = 0.0;
                        self.recognition_updates = 0;
                        self.recognition_window_started = Instant::now();
                        self.last_automation_rule = None;
                        self.last_planned_input = None;
                    }
                    self.push_log(match state {
                        ConnectionState::Disconnected => "映像接続を終了しました",
                        ConnectionState::Connecting => "映像接続を開始しています",
                        ConnectionState::Connected => "映像socketへ接続しました",
                        ConnectionState::Disconnecting => "映像接続を終了しています",
                    });
                }
                RuntimeEvent::VideoBytesReceived(total_bytes) => {
                    self.video_bytes_received = total_bytes;
                }
                RuntimeEvent::InputQueued(_) => {}
                RuntimeEvent::InputExecuted(command) => {
                    self.push_log(format!("入力を実行しました: {}", describe_input(command)));
                }
                RuntimeEvent::DetectionsUpdated(detections) => {
                    self.detections = detections;
                    self.recognition_updates = self.recognition_updates.saturating_add(1);
                    let elapsed = self.recognition_window_started.elapsed();
                    if elapsed >= Duration::from_secs(1) {
                        self.recognition_fps =
                            self.recognition_updates as f32 / elapsed.as_secs_f32();
                        self.recognition_updates = 0;
                        self.recognition_window_started = Instant::now();
                    }
                }
                RuntimeEvent::AutomationProfileChanged { name, path } => {
                    self.automation_profile_name = Some(name.clone());
                    self.automation_profile_path = path.to_string_lossy().into_owned();
                    self.profile_editor.set_path(&self.automation_profile_path);
                    self.config.automation_profile_path = Some(path);
                    self.save_config();
                    self.push_log(format!("自動化profileを変更しました: {name}"));
                }
                RuntimeEvent::AutomationProfileValidated {
                    name,
                    path,
                    templates,
                    rules,
                } => {
                    let summary = format!("{name} / {templates} templates / {rules} rules");
                    self.last_profile_validation = Some(summary.clone());
                    self.push_log(format!(
                        "profileを検証しました: {} / {summary}",
                        path.display()
                    ));
                }
                RuntimeEvent::AutomationProfileEditorLoaded { path, profile } => {
                    let name = profile.name.clone();
                    self.profile_editor.loaded(path, profile);
                    self.push_log(format!("rule editorへ{name}を読み込みました"));
                }
                RuntimeEvent::AutomationProfileSaved {
                    name,
                    path,
                    templates,
                    rules,
                } => {
                    self.profile_editor.saved(path.clone(), &name);
                    self.automation_profile_path = path.to_string_lossy().into_owned();
                    self.push_log(format!(
                        "profileを保存しました: {name} / {templates} templates / {rules} rules"
                    ));
                }
                RuntimeEvent::AutomationDryRunChanged(enabled) => {
                    self.automation_dry_run = enabled;
                    self.config.automation_dry_run = enabled;
                    self.save_config();
                    self.push_log(if enabled {
                        "dry-runを有効にしました"
                    } else {
                        "dry-runを無効にしました"
                    });
                }
                RuntimeEvent::AutomationRuleFired(rule_id) => {
                    self.last_automation_rule = Some(rule_id.clone());
                    self.push_log(format!("Ruleを実行しました: {rule_id}"));
                }
                RuntimeEvent::AutomationLog(message) => {
                    self.push_log(format!("自動化: {message}"));
                }
                RuntimeEvent::AutomationInputPlanned { rule_id, command } => {
                    let description = describe_normalized_input(command);
                    self.last_planned_input = Some(description.clone());
                    self.push_log(format!("dry-run: {rule_id} / {description}"));
                }
                RuntimeEvent::OfflineAutomationStarted => {
                    self.offline_automation_running = true;
                    self.last_automation_rule = None;
                    self.last_planned_input = None;
                    self.push_log("オフライン実行を開始しました");
                }
                RuntimeEvent::OfflineAutomationFinished {
                    processed_frames,
                    stopped,
                } => {
                    self.offline_automation_running = false;
                    self.push_log(if stopped {
                        format!("オフライン実行を停止しました: {processed_frames} frames")
                    } else {
                        format!("オフライン実行が完了しました: {processed_frames} frames")
                    });
                }
                RuntimeEvent::Error(message) => {
                    error!(%message, "runtime error");
                    self.push_log(format!("エラー: {message}"));
                }
            }
        }

        let latest_frame = self
            .runtime
            .as_ref()
            .and_then(AppRuntime::take_latest_frame);
        if let Some(frame) = latest_frame {
            self.update_preview_texture(context, &frame);
        }
    }

    fn update_preview_texture(&mut self, context: &egui::Context, frame: &Frame) {
        let size = [frame.width() as usize, frame.height() as usize];
        let image = match frame.format() {
            PixelFormat::Rgb8 => egui::ColorImage::from_rgb(size, frame.pixels()),
            PixelFormat::Rgba8 => egui::ColorImage::from_rgba_unmultiplied(size, frame.pixels()),
        };
        if let Some(texture) = self.preview_texture.as_mut() {
            texture.set(image, egui::TextureOptions::LINEAR);
        } else {
            self.preview_texture =
                Some(context.load_texture("android-preview", image, egui::TextureOptions::LINEAR));
        }
        self.preview_resolution = Some(size);
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

    fn save_config(&mut self) {
        if let Err(error) = self.config.save(PathBuf::from("better-e7.toml")) {
            self.push_log(format!("設定の保存に失敗しました: {error}"));
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
                    && !self.offline_automation_running
                    && match self.connection_state {
                        ConnectionState::Disconnected => self.selected_device.is_some(),
                        ConnectionState::Connected => true,
                        ConnectionState::Connecting | ConnectionState::Disconnecting => false,
                    };
                let label = match self.connection_state {
                    ConnectionState::Disconnected => "開始",
                    ConnectionState::Connected => "停止",
                    ConnectionState::Connecting | ConnectionState::Disconnecting => "処理中",
                };
                if ui
                    .add_enabled(can_toggle, egui::Button::new(label))
                    .clicked()
                {
                    let command = match self.connection_state {
                        ConnectionState::Disconnected => RuntimeCommand::StartAutomation,
                        ConnectionState::Connected => RuntimeCommand::StopAutomation,
                        ConnectionState::Connecting | ConnectionState::Disconnecting => return,
                    };
                    self.send(command);
                }
            });
        });
    }

    fn show_devices(&mut self, context: &egui::Context) {
        let mut input_command = None;
        let mut profile_path = None;
        let mut validation_path = None;
        let mut dry_run = None;
        let mut offline_command = None;
        let mut open_editor = false;
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
                            device.is_ready()
                                && self.connection_state == ConnectionState::Disconnected,
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
                ui.heading("入力");
                let input_enabled = self.connection_state == ConnectionState::Connected;
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(input_enabled, egui::Button::new("Home"))
                        .clicked()
                    {
                        input_command = Some(InputCommand::Key {
                            android_key_code: 3,
                        });
                    }
                    if ui
                        .add_enabled(input_enabled, egui::Button::new("Back"))
                        .clicked()
                    {
                        input_command = Some(InputCommand::Key {
                            android_key_code: 4,
                        });
                    }
                });
                if ui
                    .add_enabled(input_enabled, egui::Button::new("上へswipe"))
                    .clicked()
                    && let (Ok(from), Ok(to)) = (
                        NormalizedPoint::new(0.5, 0.8),
                        NormalizedPoint::new(0.5, 0.2),
                    )
                {
                    input_command = Some(InputCommand::Swipe {
                        from,
                        to,
                        duration: Duration::from_millis(300),
                    });
                }
                ui.label("previewをclickすると端末をtapします");

                ui.separator();
                ui.heading("タスク");
                ui.label(format!(
                    "profile: {}",
                    self.automation_profile_name.as_deref().unwrap_or("未設定")
                ));
                let can_configure = self.runtime.is_some()
                    && self.connection_state == ConnectionState::Disconnected
                    && !self.offline_automation_running;
                ui.add_enabled(
                    can_configure,
                    egui::TextEdit::singleline(&mut self.automation_profile_path)
                        .hint_text("automation.toml"),
                );
                if ui
                    .add_enabled(can_configure, egui::Button::new("profileを読み込む"))
                    .clicked()
                {
                    let path = self.automation_profile_path.trim();
                    if !path.is_empty() {
                        profile_path = Some(PathBuf::from(path));
                    }
                }
                if ui
                    .add_enabled(can_configure, egui::Button::new("profileを検証"))
                    .clicked()
                {
                    let path = self.automation_profile_path.trim();
                    if !path.is_empty() {
                        validation_path = Some(PathBuf::from(path));
                    }
                }
                ui.label(format!(
                    "検証結果: {}",
                    self.last_profile_validation.as_deref().unwrap_or("未実行")
                ));
                if ui
                    .add_enabled(can_configure, egui::Button::new("rule editorを開く"))
                    .clicked()
                {
                    open_editor = true;
                }
                let mut enabled = self.automation_dry_run;
                if ui
                    .add_enabled(can_configure, egui::Checkbox::new(&mut enabled, "dry-run"))
                    .changed()
                {
                    dry_run = Some(enabled);
                }
                ui.label("dry-runではAndroidへ入力を送りません");
                ui.label(format!(
                    "history: {}",
                    self.config
                        .automation_history_path
                        .as_ref()
                        .map(|path| path.to_string_lossy())
                        .as_deref()
                        .unwrap_or("無効")
                ));

                ui.separator();
                ui.heading("オフライン実行");
                ui.add_enabled(
                    !self.offline_automation_running,
                    egui::TextEdit::singleline(&mut self.offline_frames_directory)
                        .hint_text("frames directory"),
                );
                let can_start_offline = self.runtime.is_some()
                    && self.connection_state == ConnectionState::Disconnected
                    && !self.offline_automation_running
                    && !self.automation_profile_path.trim().is_empty()
                    && !self.offline_frames_directory.trim().is_empty();
                let offline_button_enabled = can_start_offline || self.offline_automation_running;
                let offline_button_label = if self.offline_automation_running {
                    "停止"
                } else {
                    "保存Frameを実行"
                };
                if ui
                    .add_enabled(
                        offline_button_enabled,
                        egui::Button::new(offline_button_label),
                    )
                    .clicked()
                {
                    offline_command = Some(if self.offline_automation_running {
                        RuntimeCommand::StopOfflineAutomation
                    } else {
                        RuntimeCommand::StartOfflineAutomation {
                            profile_path: PathBuf::from(self.automation_profile_path.trim()),
                            frames_directory: PathBuf::from(self.offline_frames_directory.trim()),
                            history_path: self.config.automation_history_path.clone(),
                        }
                    });
                }
                ui.label("PNG / JPEGを名前順にdry-runします");
            });
        if let Some(command) = input_command {
            self.send(RuntimeCommand::SubmitInput(command));
        }
        if let Some(path) = profile_path {
            self.send(RuntimeCommand::LoadAutomationProfile(path));
        }
        if let Some(path) = validation_path {
            self.last_profile_validation = None;
            self.send(RuntimeCommand::ValidateAutomationProfile(path));
        }
        if let Some(enabled) = dry_run {
            self.send(RuntimeCommand::SetAutomationDryRun(enabled));
        }
        if let Some(command) = offline_command {
            self.send(command);
        }
        if open_editor {
            self.profile_editor.set_path(&self.automation_profile_path);
            self.profile_editor.open();
        }
    }

    fn show_preview(&mut self, context: &egui::Context) {
        let mut input_command = None;
        egui::CentralPanel::default().show(context, |ui| {
            ui.heading("プレビュー");
            let available = ui.available_size();
            let preview_height = (available.y - 120.0).max(160.0);
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(available.x, preview_height),
                egui::Sense::click(),
            );
            ui.painter()
                .rect_filled(rect, 6.0, egui::Color32::from_gray(24));
            if let Some(texture) = &self.preview_texture {
                let texture_size = texture.size_vec2();
                let scale = (rect.width() / texture_size.x).min(rect.height() / texture_size.y);
                let image_rect = egui::Rect::from_center_size(rect.center(), texture_size * scale);
                ui.painter().image(
                    texture.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                self.paint_detections(ui.painter(), image_rect);
                if self.connection_state == ConnectionState::Connected
                    && response.clicked()
                    && let Some(position) = response.interact_pointer_pos()
                    && image_rect.contains(position)
                {
                    let x = (position.x - image_rect.min.x) / image_rect.width();
                    let y = (position.y - image_rect.min.y) / image_rect.height();
                    if let Ok(point) = NormalizedPoint::new(x, y) {
                        input_command = Some(InputCommand::Tap { point });
                    }
                }
            } else {
                let preview_message = match self.connection_state {
                    ConnectionState::Disconnected => "Android映像は未接続です",
                    ConnectionState::Connecting => "Android映像へ接続しています",
                    ConnectionState::Connected => "最初の映像frameを待っています",
                    ConnectionState::Disconnecting => "Android映像を切断しています",
                };
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    preview_message,
                    egui::FontId::proportional(22.0),
                    egui::Color32::GRAY,
                );
            }
            ui.separator();
            ui.heading("状態");
            ui.label(format!(
                "端末: {}",
                self.selected_device.as_deref().unwrap_or("未選択")
            ));
            ui.label(format!("認識FPS: {:.1}", self.recognition_fps));
            ui.label(format!("検出数: {}", self.detections.len()));
            ui.label(format!("受信量: {} bytes", self.video_bytes_received));
            ui.label(if self.offline_automation_running {
                "オフライン実行: 実行中"
            } else {
                "オフライン実行: 停止中"
            });
            let resolution = self
                .preview_resolution
                .map(|[width, height]| format!("{width} x {height}"))
                .unwrap_or_else(|| "--".to_owned());
            ui.label(format!("映像解像度: {resolution}"));
            ui.label(format!(
                "最後のRule: {}",
                self.last_automation_rule.as_deref().unwrap_or("未実行")
            ));
            ui.label(format!(
                "予定入力: {}",
                self.last_planned_input.as_deref().unwrap_or("なし")
            ));
        });
        if let Some(command) = input_command {
            self.send(RuntimeCommand::SubmitInput(command));
        }
    }

    fn paint_detections(&self, painter: &egui::Painter, image_rect: egui::Rect) {
        let color = egui::Color32::from_rgb(80, 220, 120);
        let stroke = egui::Stroke::new(2.0, color);
        for detection in &self.detections {
            let bounds = detection.bounds;
            let min = egui::pos2(
                image_rect.min.x + bounds.left() * image_rect.width(),
                image_rect.min.y + bounds.top() * image_rect.height(),
            );
            let max = egui::pos2(
                image_rect.min.x + bounds.right() * image_rect.width(),
                image_rect.min.y + bounds.bottom() * image_rect.height(),
            );
            let top_right = egui::pos2(max.x, min.y);
            let bottom_left = egui::pos2(min.x, max.y);
            painter.line_segment([min, top_right], stroke);
            painter.line_segment([top_right, max], stroke);
            painter.line_segment([max, bottom_left], stroke);
            painter.line_segment([bottom_left, min], stroke);
            painter.text(
                min + egui::vec2(2.0, -2.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{} {:.0}%", detection.label, detection.confidence * 100.0),
                egui::FontId::monospace(13.0),
                color,
            );
        }
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

fn describe_input(command: PixelInputCommand) -> String {
    match command {
        PixelInputCommand::Tap { x, y } => format!("tap {x} {y}"),
        PixelInputCommand::Swipe {
            from_x,
            from_y,
            to_x,
            to_y,
            duration,
        } => format!(
            "swipe {from_x} {from_y} {to_x} {to_y} {}ms",
            duration.as_millis()
        ),
        PixelInputCommand::Key { android_key_code } => {
            format!("keyevent {android_key_code}")
        }
    }
}

fn describe_normalized_input(command: InputCommand) -> String {
    match command {
        InputCommand::Tap { point } => {
            format!("tap {:.3} {:.3}", point.x(), point.y())
        }
        InputCommand::Swipe { from, to, duration } => format!(
            "swipe {:.3} {:.3} {:.3} {:.3} {}ms",
            from.x(),
            from.y(),
            to.x(),
            to.y(),
            duration.as_millis()
        ),
        InputCommand::Key { android_key_code } => {
            format!("keyevent {android_key_code}")
        }
    }
}

impl eframe::App for BetterE7App {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_runtime_events(context);
        self.show_toolbar(context);
        self.show_devices(context);
        self.show_preview(context);
        self.show_logs(context);
        let editor_enabled = self.runtime.is_some()
            && self.connection_state == ConnectionState::Disconnected
            && !self.offline_automation_running;
        if let Some(command) = self.profile_editor.show(context, editor_enabled) {
            match command {
                ProfileEditorCommand::Load(path) => {
                    self.send(RuntimeCommand::LoadAutomationProfileEditor(path));
                }
                ProfileEditorCommand::Save { path, profile } => {
                    self.send(RuntimeCommand::SaveAutomationProfile { path, profile });
                }
            }
        }
        context.request_repaint_after(Duration::from_millis(100));
    }
}
