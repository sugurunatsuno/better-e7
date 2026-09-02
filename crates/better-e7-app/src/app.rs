use std::time::Duration;

use better_e7_adb::AdbDevice;
use better_e7_config::AppConfig;
use better_e7_core::{Frame, InputCommand, NormalizedPoint, PixelFormat, PixelInputCommand};
use better_e7_runtime::{
    AppRuntime, AutomationState, ConnectionState, RuntimeCommand, RuntimeEvent,
};
use eframe::egui;
use tracing::{error, info};

const MAX_VISIBLE_LOGS: usize = 500;

pub struct BetterE7App {
    runtime: Option<AppRuntime>,
    devices: Vec<AdbDevice>,
    selected_device: Option<String>,
    automation_state: AutomationState,
    connection_state: ConnectionState,
    video_bytes_received: u64,
    preview_texture: Option<egui::TextureHandle>,
    preview_resolution: Option<[usize; 2]>,
    logs: Vec<String>,
}

impl BetterE7App {
    pub fn new(creation_context: &eframe::CreationContext<'_>, config: AppConfig) -> Self {
        install_japanese_font(&creation_context.egui_ctx);
        let mut app = Self {
            runtime: None,
            devices: Vec::new(),
            selected_device: None,
            automation_state: AutomationState::Stopped,
            connection_state: ConnectionState::Disconnected,
            video_bytes_received: 0,
            preview_texture: None,
            preview_resolution: None,
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
                ui.label("ゲームプラグインは未実装です");
            });
        if let Some(command) = input_command {
            self.send(RuntimeCommand::SubmitInput(command));
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
            ui.label("認識FPS: --");
            ui.label(format!("受信量: {} bytes", self.video_bytes_received));
            let resolution = self
                .preview_resolution
                .map(|[width, height]| format!("{width} x {height}"))
                .unwrap_or_else(|| "--".to_owned());
            ui.label(format!("映像解像度: {resolution}"));
            ui.label("ゲーム状態: Unknown");
        });
        if let Some(command) = input_command {
            self.send(RuntimeCommand::SubmitInput(command));
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

fn install_japanese_font(context: &egui::Context) {
    let Some(bytes) = japanese_font_paths()
        .iter()
        .find_map(|path| std::fs::read(path).ok())
    else {
        tracing::warn!("Japanese font not found; text may render as tofu");
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "japanese".to_owned(),
        egui::FontData::from_owned(bytes).into(),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        if let Some(fonts) = fonts.families.get_mut(&family) {
            fonts.insert(0, "japanese".to_owned());
        }
    }
    context.set_fonts(fonts);
}

#[cfg(target_os = "macos")]
fn japanese_font_paths() -> &'static [&'static str] {
    &[
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        "/System/Library/Fonts/STHeiti Medium.ttc",
    ]
}

#[cfg(target_os = "windows")]
fn japanese_font_paths() -> &'static [&'static str] {
    &[
        r"C:\Windows\Fonts\meiryo.ttc",
        r"C:\Windows\Fonts\YuGothM.ttc",
    ]
}

#[cfg(target_os = "linux")]
fn japanese_font_paths() -> &'static [&'static str] {
    &[
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
    ]
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

impl eframe::App for BetterE7App {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_runtime_events(context);
        self.show_toolbar(context);
        self.show_devices(context);
        self.show_logs(context);
        self.show_preview(context);
        context.request_repaint_after(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::japanese_font_paths;

    #[test]
    fn has_japanese_font_candidates() {
        assert!(!japanese_font_paths().is_empty());
    }
}
