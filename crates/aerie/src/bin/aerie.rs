#![cfg(feature = "ui")]
use clap::Parser as _;

use aerie::config::Args;

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let settings_path = args.config.clone().unwrap_or(
        dirs::config_dir()
            .map(|p| p.join("aerie"))
            .unwrap_or_default()
            .join("settings.toml"),
    );

    // Shhh...
    let _ = dotenvy::from_path(settings_path.with_file_name(".env"));

    if let Some(env_handle) = &args.env {
        let _ = if env_handle.to_str() == Some("-") {
            dotenvy::from_read(std::io::stdin())
        } else {
            dotenvy::from_path(env_handle)
        };
    }

    let app = aerie::app::App::builder()
        .name("aerie")
        .args(args)
        .settings_path(settings_path)
        .min_size(egui::vec2(800.0, 400.0))
        .build();
    app.run_app()?;

    Ok(())
}
