use std::{
    borrow::Cow,
    collections::BTreeMap,
    fs::OpenOptions,
    hash::DefaultHasher,
    path::{Path, PathBuf},
    sync::Arc,
};

use arc_swap::ArcSwap;
use itertools::Itertools;

use crate::workflow::ShadowGraph;

use super::WorkNode;

type WorkGraph = ShadowGraph<WorkNode>;

pub trait WorkflowStore {
    // type Graph: std::fmt::Debug + Clone;
    fn load(&mut self, name: &str) -> anyhow::Result<WorkGraph>;
    fn save(&mut self, name: &str, value: WorkGraph) -> anyhow::Result<()>;
    fn names(&self) -> impl Iterator<Item = Cow<'_, str>>;
    fn exists(&self, key: &str) -> bool;
    fn description(&'_ self, key: &str) -> Cow<'_, str>;
    fn schema(&'_ self, key: &str) -> Cow<'_, str>;

    fn remove(&mut self, key: &str) -> anyhow::Result<()>;
    fn rename(&mut self, old_name: &str, new_name: &str) -> anyhow::Result<()>;
    fn backup(&self, name: &str) -> anyhow::Result<()>;

    /// Fetches from cache without loading
    fn get(&self, key: &str) -> Option<WorkGraph>;

    /// Puts into cache without saving
    fn put(&mut self, key: &str, value: WorkGraph);
}

/// Handles persistence of workflows
#[derive(Default, Debug, Clone)]
pub struct WorkflowStoreFile {
    path: PathBuf,

    /// Cache of loaded workflows
    workflows: BTreeMap<String, WorkGraph>,
}

impl WorkflowStoreFile {
    #[deprecated]
    /// Loads all workflows into memory
    pub fn load_all(path: impl AsRef<Path>, tutorial: bool) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let workflows: BTreeMap<String, WorkGraph> = if path.is_file() {
            let reader = OpenOptions::new().read(true).open(&path)?;
            serde_yml::from_reader(reader)?
        } else {
            let mut result: BTreeMap<String, WorkGraph> = Default::default();
            result.insert("".into(), Default::default());

            if tutorial {
                let bytes = include_bytes!("../../tutorial/workflows/basic.yml");
                if let Ok(graph) = serde_yml::from_slice::<WorkGraph>(bytes) {
                    result.insert("basic".into(), graph);
                }

                let bytes = include_bytes!("../../tutorial/workflows/chatty.yml");
                if let Ok(graph) = serde_yml::from_slice::<WorkGraph>(bytes) {
                    result.insert("chatty".into(), graph);
                }
            }

            result
        };

        Ok(Self { path, workflows })
    }

    #[deprecated]
    pub fn save_all(&self) -> anyhow::Result<()> {
        tracing::info!("Saving all workflows to {:?}", &self.path);
        let writer = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path)?;

        serde_yml::to_writer(writer, &self.workflows)?;

        Ok(())
    }
}

impl WorkflowStore for WorkflowStoreFile {
    // type Graph = WorkGraph;

    fn load(&mut self, name: &str) -> anyhow::Result<WorkGraph> {
        // no-op since loaded in bulk
        self.get(name).ok_or(anyhow::anyhow!("Not found"))
    }

    fn save(&mut self, name: &str, value: WorkGraph) -> anyhow::Result<()> {
        self.put(name, value.clone());

        #[allow(deprecated)]
        self.save_all()
    }

    fn names(&self) -> impl Iterator<Item = Cow<'_, str>> {
        self.workflows.keys().map(|s| Cow::Borrowed(s.as_str()))
    }

    fn exists(&self, key: &str) -> bool {
        self.workflows.contains_key(key)
    }

    fn description(&self, key: &str) -> Cow<'_, str> {
        self.get(key)
            .map(|g| Cow::Owned(g.description.to_string()))
            .unwrap_or(Cow::Owned(String::new()))
    }

    fn schema(&self, key: &str) -> Cow<'_, str> {
        self.get(key)
            .map(|g| Cow::Owned(g.schema.to_string()))
            .unwrap_or(Cow::Owned(String::new()))
    }

    fn get(&self, key: &str) -> Option<WorkGraph> {
        self.workflows.get(key).cloned()
    }

    fn put(&mut self, key: &str, value: WorkGraph) {
        self.workflows.insert(key.into(), value);
    }

    fn remove(&mut self, key: &str) -> anyhow::Result<()> {
        self.workflows.remove(key);

        #[allow(deprecated)]
        self.save_all()?;

        Ok(())
    }

    fn rename(&mut self, old_name: &str, new_name: &str) -> anyhow::Result<()> {
        if let Some(value) = self.workflows.remove(old_name) {
            self.put(new_name, value);
        }

        Ok(())
    }

    fn backup(&self, name: &str) -> anyhow::Result<()> {
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

#[derive(Default, Debug, Clone)]
pub struct WorkflowStoreDir {
    path: PathBuf,

    /// Cache of loaded workflows
    cache: Arc<ArcSwap<im::OrdMap<String, WorkGraph>>>,
}

impl WorkflowStoreDir {
    pub fn load_all(dir: impl AsRef<Path>, tutorial: bool) -> anyhow::Result<Self> {
        let mut workflows = im::OrdMap::default();
        let path = dir.as_ref().to_path_buf();

        let paths = std::fs::read_dir(&path)?
            .filter_map(|f| f.ok())
            .map(|f| f.path())
            .filter(|p| p.is_file())
            .filter(|p| matches!(p.extension(), Some(x) if x == "yml"));

        for path in paths {
            let Some(name) = path
                .file_stem()
                .and_then(|s| s.to_os_string().into_string().ok())
            else {
                continue;
            };

            let reader = OpenOptions::new().read(true).open(&path)?;
            workflows.insert(name, serde_yml::from_reader(reader)?);
        }

        let mut this = Self {
            path,
            cache: Arc::new(ArcSwap::from_pointee(workflows)),
        };

        tracing::info!("Loaded all workflows: {:?}", this.names().collect_vec());

        if tutorial && this.names().all(|n| n == "__default__") {
            let bytes = include_bytes!("../../tutorial/workflows/basic.yml");
            if let Ok(graph) = serde_yml::from_slice::<WorkGraph>(bytes) {
                let _ = this.save("basic", graph);
            }

            let bytes = include_bytes!("../../tutorial/workflows/chatty.yml");
            if let Ok(graph) = serde_yml::from_slice::<WorkGraph>(bytes) {
                let _ = this.save("chatty", graph);
            }
        }

        this.cache
            .rcu(|cache| cache.update("".into(), Default::default()));

        Ok(this)
    }

    #[deprecated]
    pub fn save_all(&self) -> anyhow::Result<()> {
        let writer = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path)?;

        serde_yml::to_writer(writer, &self.cache)?;

        Ok(())
    }
}

impl WorkflowStore for WorkflowStoreDir {
    // type Graph = WorkGraph;

    fn load(&mut self, name: &str) -> anyhow::Result<WorkGraph> {
        if let Some(graph) = self.cache.load().get(name) {
            return Ok(graph.clone());
        }

        let path = self.path.join(name).with_extension("yml");
        let file = OpenOptions::new().read(true).open(path)?;

        let shadow_graph: WorkGraph = serde_yml::from_reader(file)?;
        self.cache
            .rcu(|cache| cache.update(name.to_string(), shadow_graph.clone()));

        Ok(shadow_graph)
    }

    fn save(&mut self, name: &str, value: WorkGraph) -> anyhow::Result<()> {
        if !name.is_empty() {
            self.cache
                .rcu(|cache| cache.update(name.to_string(), value.clone()));

            let path = self.path.join(name).with_extension("yml");
            tracing::info!("Saving {name} to {path:?}: {value:?}");
            let writer = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)?;

            serde_yml::to_writer(writer, &value)?;
        }

        Ok(())
    }

    fn names(&self) -> impl Iterator<Item = Cow<'_, str>> {
        glob::glob(&self.path.join("*.yml").display().to_string())
            .unwrap()
            .filter_map(|p| p.ok())
            .filter_map(|p| p.file_stem().map(|stem| stem.display().to_string()))
            .map(Cow::Owned)
    }

    fn exists(&self, key: &str) -> bool {
        self.cache.load().contains_key(key) || self.path.join(key).with_extension("yml").exists()
    }

    fn description(&self, key: &str) -> Cow<'_, str> {
        self.get(key)
            .map(|g| Cow::Owned(g.description.to_string()))
            .unwrap_or(Cow::Owned(String::new()))
    }

    fn schema(&self, key: &str) -> Cow<'_, str> {
        self.get(key)
            .map(|g| Cow::Owned(g.schema.to_string()))
            .unwrap_or(Cow::Owned(String::new()))
    }

    // TODO: deprecate
    fn get(&self, key: &str) -> Option<WorkGraph> {
        self.cache.load().get(key).cloned()
    }

    fn put(&mut self, key: &str, value: WorkGraph) {
        self.cache
            .rcu(|cache| cache.update(key.into(), value.clone()));
    }

    fn remove(&mut self, key: &str) -> anyhow::Result<()> {
        self.cache.rcu(|cache| cache.without(key));
        let path = self.path.join(key).with_extension("yml");
        std::fs::remove_file(path)?;

        Ok(())
    }

    fn rename(&mut self, old_name: &str, new_name: &str) -> anyhow::Result<()> {
        self.cache.rcu(|cache| {
            if let Some(value) = cache.get(old_name) {
                cache
                    .without(old_name)
                    .update(new_name.to_string(), value.clone())
            } else {
                cache.as_ref().clone()
            }
        });

        let old_path = self.path.join(old_name).with_extension("yml");
        let new_path = self.path.join(new_name).with_extension("yml");
        std::fs::rename(old_path, new_path)?;

        Ok(())
    }

    fn backup(&self, name: &str) -> anyhow::Result<()> {
        use std::hash::{Hash, Hasher};

        tracing::info!("Creating backup for {name}");
        if let Some(graph) = self.cache.load().get(name)
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
