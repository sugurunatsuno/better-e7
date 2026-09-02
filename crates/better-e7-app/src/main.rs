mod app;
mod profile_editor;

use std::path::PathBuf;

use app::BetterE7App;
use better_e7_config::AppConfig;
use eframe::egui;
use tracing::{error, info};

fn main() -> eframe::Result {
    init_logging();
    let config_path = PathBuf::from("better-e7.toml");
    let config = match AppConfig::load_or_create(&config_path) {
        Ok(config) => {
            info!(path = %config_path.display(), "configuration loaded");
            config
        }
        Err(error) => {
            error!(%error, "failed to load configuration; using defaults");
            AppConfig::default()
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1120.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        "better-e7",
        options,
        Box::new(move |creation_context| Ok(Box::new(BetterE7App::new(creation_context, config)))),
    )
}

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .try_init();
}
