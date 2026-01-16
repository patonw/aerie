use clap::Parser as _;

use aerie::config::Args;

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let settings_path = args.config.clone().unwrap_or(
        dirs::config_dir()
            .map(|p| p.join("aerie"))
            .unwrap_or_default()
            .join("workbench.yml"),
    );

    // Shhh...
    let _ = dotenvy::from_path(settings_path.with_file_name(".env"));

    let app = aerie::app::App::builder()
        .name("aerie")
        .args(args)
        .settings_path(settings_path)
        .min_size(egui::vec2(800.0, 400.0))
        .build();
    app.run_app()?;

    Ok(())
}
