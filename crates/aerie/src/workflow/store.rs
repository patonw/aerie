use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use egui_snarl::Snarl;

use super::WorkNode;

#[derive(Default, Debug, Clone)]
pub struct WorkflowStore {
    pub path: PathBuf,
    pub workflows: BTreeMap<String, Snarl<WorkNode>>,
}

impl WorkflowStore {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let workflows = if path.is_file() {
            let reader = OpenOptions::new().read(true).open(&path)?;
            serde_yml::from_reader(reader)?
        } else {
            Default::default()
        };

        let mut this = Self { path, workflows };
        this.fixup();
        Ok(this)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let writer = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path)?;

        serde_yml::to_writer(writer, &self.workflows)?;

        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<&Snarl<WorkNode>> {
        self.workflows.get(key)
    }

    pub fn put(&mut self, key: &str, value: Snarl<WorkNode>) {
        self.workflows.insert(key.into(), value);
    }

    pub fn fixup(&mut self) {
        if self.workflows.is_empty() {
            self.workflows
                .insert("default".to_string(), Snarl::default());
        }

        for snarl in self.workflows.values_mut() {
            tracing::info!("Examining {snarl:?}");
            if snarl.nodes().count() < 1 || !snarl.nodes().any(|n| matches!(n, WorkNode::Start(_)))
            {
                tracing::info!("Missing start");
                snarl.insert_node(egui::pos2(0.0, 0.0), WorkNode::Start(Default::default()));
            }
        }
    }
}
