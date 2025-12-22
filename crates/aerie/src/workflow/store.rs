use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    hash::DefaultHasher,
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
    pub fn load(path: impl AsRef<Path>, tutorial: bool) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let workflows: BTreeMap<String, ShadowGraph<WorkNode>> = if path.is_file() {
            let reader = OpenOptions::new().read(true).open(&path)?;
            serde_yml::from_reader(reader)?
        } else {
            let mut result: BTreeMap<String, ShadowGraph<WorkNode>> = Default::default();
            let bytes = include_bytes!("../../tutorial/workflows/__default__.yml");
            if let Ok(graph) = serde_yml::from_slice::<ShadowGraph<WorkNode>>(bytes) {
                result.insert("basic".into(), graph);
            }

            if tutorial {
                let bytes = include_bytes!("../../tutorial/workflows/basic.yml");
                if let Ok(graph) = serde_yml::from_slice::<ShadowGraph<WorkNode>>(bytes) {
                    result.insert("basic".into(), graph);
                }

                let bytes = include_bytes!("../../tutorial/workflows/chatty.yml");
                if let Ok(graph) = serde_yml::from_slice::<ShadowGraph<WorkNode>>(bytes) {
                    result.insert("chatty".into(), graph);
                }
            }

            result
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

    pub fn get(&self, key: &str) -> Option<ShadowGraph<WorkNode>> {
        self.workflows.get(key).cloned()
    }

    pub fn get_snarl(&self, key: &str) -> Option<Snarl<WorkNode>> {
        self.workflows
            .get(key)
            .map(|it| Snarl::try_from(it.clone()).unwrap())
    }

    pub fn put(&mut self, key: &str, value: ShadowGraph<WorkNode>) {
        self.workflows.insert(key.into(), value);
    }
    pub fn remove(&mut self, key: &str) {
        self.workflows.remove(key);
    }

    pub fn rename(&mut self, old_name: &str, new_name: &str) {
        if let Some(value) = self.workflows.remove(old_name) {
            self.put(new_name, value);
        }
    }

    pub fn create_backup(&self, name: &str) -> anyhow::Result<()> {
        use std::hash::{Hash, Hasher};

        tracing::info!("Creating backup for {name}");
        if let Some(graph) = self.workflows.get(name)
            && let Some(dir) = self.path.parent()
        {
            let dir = dir.join("backups");
            std::fs::create_dir_all(&dir)?;

            let mut s = DefaultHasher::new();
            graph.hash(&mut s);
            let hash = s.finish();

            // let dt = chrono::offset::Local::now();
            // let ts = dt.format("%Y-%m-%dT%H:%M:%S").to_string();
            let file = dir.join(format!("{name}-{hash:x}.yml"));
            tracing::info!("Backup location {file:?}");

            let writer = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&file)?;

            serde_yml::to_writer(writer, graph)?;
        }

        Ok(())
    }
}
