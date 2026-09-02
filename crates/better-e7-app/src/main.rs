use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1120.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        "better-e7",
        options,
        Box::new(|creation_context| Ok(Box::new(BetterE7App::new(creation_context)))),
    )
}

#[derive(Debug, Default)]
struct BetterE7App {
    running: bool,
    selected_device: Option<String>,
}

impl BetterE7App {
    fn new(_creation_context: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }
}

impl eframe::App for BetterE7App {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("toolbar").show(context, |ui| {
            ui.horizontal(|ui| {
                ui.heading("better-e7");
                ui.separator();
                let status = if self.running {
                    "実行中"
                } else {
                    "停止中"
                };
                ui.label(status);
                if ui
                    .button(if self.running { "停止" } else { "開始" })
                    .clicked()
                {
                    self.running = !self.running;
                }
            });
        });

        egui::SidePanel::left("devices")
            .resizable(true)
            .default_width(240.0)
            .show(context, |ui| {
                ui.heading("端末");
                ui.label("ADB端末の検出は未実装です");
                if ui.button("デモ端末を選択").clicked() {
                    self.selected_device = Some("demo-device".to_owned());
                }
                ui.separator();
                ui.heading("タスク");
                ui.label("ゲームプラグインの読み込みは未実装です");
            });

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
                "Android映像",
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

        egui::TopBottomPanel::bottom("logs")
            .resizable(true)
            .default_height(110.0)
            .show(context, |ui| {
                ui.heading("ログ");
                ui.monospace("better-e7を起動しました");
            });
    }
}
