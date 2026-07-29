use std::rc::Rc;

use aerie::{
    config::Args,
    egui, inventory, snarl, typetag,
    workflow::{DynNode, FlexNode, UiNode, WorkNode, nodes::GraphSubmenu},
};
use clap::Parser as _;
use serde::{Deserialize, Serialize};

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let settings_path = args.config.clone().unwrap_or(
        // Change me or not
        dirs::config_dir()
            .map(|p| p.join("aerie"))
            .unwrap_or_default()
            .join("settings.toml"),
    );

    let app = aerie::app::App::builder()
        .name("aerie-custom") // Changes the default data directory
        .data_dir_fn(Rc::new(|path| path.with_file_name("aerie-hello"))) // otherwise
        .args(args)
        .settings_path(settings_path)
        .min_size(egui::vec2(800.0, 400.0))
        .build();
    app.run_app()?;

    Ok(())
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub struct HelloNode {}

impl DynNode for HelloNode {
    fn title(&self) -> &str {
        "Hello World"
    }
}

impl UiNode for HelloNode {}

#[typetag::serde]
impl FlexNode for HelloNode {}

fn script_node_menu(ui: &mut egui::Ui, snarl: &mut snarl::Snarl<WorkNode>, pos: egui::Pos2) {
    ui.menu_button("hello", |ui| {
        if ui.button("Hello").clicked() {
            snarl.insert_node(pos.into(), HelloNode::default().into());
        }
    });
}

inventory::submit! {
    GraphSubmenu("hello", script_node_menu)
}
