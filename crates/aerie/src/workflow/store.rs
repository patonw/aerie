use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use egui_snarl::Snarl;

use crate::workflow::ShadowGraph;

use super::WorkNode;

#[derive(Default, Debug, Clone)]
pub struct WorkflowStore {
    pub path: PathBuf,
    pub workflows: BTreeMap<String, ShadowGraph<WorkNode>>,
}

impl WorkflowStore {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let workflows: BTreeMap<String, ShadowGraph<WorkNode>> = if path.is_file() {
            let reader = OpenOptions::new().read(true).open(&path)?;
            serde_yml::from_reader(reader)?
        } else {
            Default::default()
        };

        Ok(Self { path, workflows })
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

    pub fn get(&self, key: &str) -> Snarl<WorkNode> {
        self.workflows
            .get(key)
            .map(|it| Snarl::try_from(it.clone()).unwrap())
            .unwrap_or_default()
    }

    pub fn put(&mut self, key: &str, value: ShadowGraph<WorkNode>) {
        self.workflows.insert(key.into(), value);
    }
}
