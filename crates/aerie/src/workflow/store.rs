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
use serde_yaml_ng as serde_yml;

use crate::{storage::CachedDirStore, workflow::Workflow};

pub trait WorkflowStore {
    fn load(&mut self, name: &str) -> anyhow::Result<Workflow>;
    fn save(&mut self, name: &str, value: Workflow) -> anyhow::Result<()>;
    fn names(&self) -> impl Iterator<Item = Cow<'_, str>>;
    fn exists(&self, key: &str) -> bool;
    fn description(&'_ self, key: &str) -> Cow<'_, str>;
    fn schema(&'_ self, key: &str) -> Cow<'_, str>;

    fn remove(&mut self, key: &str) -> anyhow::Result<()>;
    fn rename(&mut self, old_name: &str, new_name: &str) -> anyhow::Result<()>;
    fn backup(&self, name: &str) -> anyhow::Result<()>;

    /// Fetches from cache without loading
    fn get(&self, key: &str) -> Option<Workflow>;

    /// Puts into cache without saving
    fn put(&mut self, key: &str, value: Workflow);
}

/// Handles persistence of workflows
#[derive(Default, Debug, Clone)]
pub struct WorkflowStoreFile {
    path: PathBuf,

    /// Cache of loaded workflows
    workflows: BTreeMap<String, Workflow>,
}

impl WorkflowStoreFile {
    #[deprecated]
    /// Loads all workflows into memory
    pub fn load_all(path: impl AsRef<Path>, tutorial: bool) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let workflows: BTreeMap<String, Workflow> = if path.is_file() {
            let reader = OpenOptions::new().read(true).open(&path)?;
            serde_yml::from_reader(reader)?
        } else {
            let mut result: BTreeMap<String, Workflow> = Default::default();
            result.insert("".into(), Default::default());

            if tutorial {
                let bytes = include_bytes!("../../tutorial/workflows/basic.yml");
                if let Ok(graph) = serde_yml::from_slice::<Workflow>(bytes) {
                    result.insert("basic".into(), graph);
                }

                let bytes = include_bytes!("../../tutorial/workflows/chatty.yml");
                if let Ok(graph) = serde_yml::from_slice::<Workflow>(bytes) {
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

    fn load(&mut self, name: &str) -> anyhow::Result<Workflow> {
        // no-op since loaded in bulk
        self.get(name).ok_or(anyhow::anyhow!("Not found"))
    }

    fn save(&mut self, name: &str, value: Workflow) -> anyhow::Result<()> {
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
            .map(|g| Cow::Owned(g.metadata.description.to_string()))
            .unwrap_or(Cow::Owned(String::new()))
    }

    fn schema(&self, key: &str) -> Cow<'_, str> {
        self.get(key)
            .map(|g| Cow::Owned(g.metadata.schema.to_string()))
            .unwrap_or(Cow::Owned(String::new()))
    }

    fn get(&self, key: &str) -> Option<Workflow> {
        self.workflows.get(key).cloned()
    }

    fn put(&mut self, key: &str, value: Workflow) {
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
    cache: Arc<ArcSwap<im::OrdMap<String, Workflow>>>,
}

impl WorkflowStoreDir {
    pub fn load_all(dir: impl AsRef<Path>, tutorial: bool) -> anyhow::Result<Self> {
        let path = dir.as_ref().to_path_buf();

        let this = Self {
            path,
            cache: Default::default(),
        };

        this.preload_all();

        let names = CachedDirStore::names(&this).collect_vec();
        tracing::info!("Loaded all workflows: {names:?}");

        if tutorial && names.iter().all(|n| n == "__default__") {
            let bytes = include_bytes!("../../tutorial/workflows/basic.yml");
            if let Ok(graph) = serde_yml::from_slice::<Workflow>(bytes) {
                let _ = CachedDirStore::save(&this, "basic", graph);
            }

            let bytes = include_bytes!("../../tutorial/workflows/chatty.yml");
            if let Ok(graph) = serde_yml::from_slice::<Workflow>(bytes) {
                let _ = CachedDirStore::save(&this, "chatty", graph);
            }
        }

        this.cache
            .rcu(|cache| cache.update("".into(), Default::default()));

        Ok(this)
    }
}

impl CachedDirStore<Workflow> for WorkflowStoreDir {
    const EXT: &'static str = "yml";

    fn base_path(&self) -> &Path {
        &self.path
    }

    fn view_cache<R>(&self, cb: impl FnOnce(&im::OrdMap<String, Workflow>) -> R) -> R {
        cb(&self.cache.load())
    }

    fn update_cache(
        &self,
        cb: impl Fn(&im::OrdMap<String, Workflow>) -> im::OrdMap<String, Workflow>,
    ) {
        self.cache.rcu(|cache| cb(cache));
    }
}

impl WorkflowStore for WorkflowStoreDir {
    // type Graph = WorkGraph;

    fn load(&mut self, name: &str) -> anyhow::Result<Workflow> {
        CachedDirStore::load(self, name)
    }

    fn save(&mut self, name: &str, value: Workflow) -> anyhow::Result<()> {
        CachedDirStore::save(self, name, value)
    }

    fn names(&self) -> impl Iterator<Item = Cow<'_, str>> {
        CachedDirStore::names(self)
    }

    fn exists(&self, key: &str) -> bool {
        CachedDirStore::exists(self, key)
    }

    fn description(&self, key: &str) -> Cow<'_, str> {
        self.get(key)
            .map(|g| Cow::Owned(g.metadata.description.to_string()))
            .unwrap_or(Cow::Owned(String::new()))
    }

    fn schema(&self, key: &str) -> Cow<'_, str> {
        self.get(key)
            .map(|g| Cow::Owned(g.metadata.schema.to_string()))
            .unwrap_or(Cow::Owned(String::new()))
    }

    // TODO: deprecate
    fn get(&self, key: &str) -> Option<Workflow> {
        CachedDirStore::get_transient(self, key)
    }

    fn put(&mut self, key: &str, value: Workflow) {
        CachedDirStore::put_cache(self, key, value)
    }

    fn remove(&mut self, key: &str) -> anyhow::Result<()> {
        CachedDirStore::remove(self, key)
    }

    fn rename(&mut self, old_name: &str, new_name: &str) -> anyhow::Result<()> {
        CachedDirStore::rename(self, old_name, new_name)
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
