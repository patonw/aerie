use std::{borrow::Cow, fs::OpenOptions, path::Path};

use itertools::Itertools as _;
use serde::{Serialize, de::DeserializeOwned};

pub trait CachedDirStore<T: Clone + Serialize + DeserializeOwned> {
    const EXT: &'static str;

    fn base_path(&self) -> &Path;
    fn view_cache<R>(&self, cb: impl FnOnce(&im::OrdMap<String, T>) -> R) -> R;
    fn update_cache(&self, cb: impl Fn(&im::OrdMap<String, T>) -> im::OrdMap<String, T>);

    fn exists(&self, key: &str) -> bool {
        self.view_cache(|cache| {
            cache.contains_key(key)
                || self
                    .base_path()
                    .join(key)
                    .with_extension(Self::EXT)
                    .exists()
        })
    }

    fn cached_names(&self) -> impl IntoIterator<Item = String> {
        self.view_cache(|cache| cache.keys().cloned().collect_vec())
    }

    fn names(&self) -> impl Iterator<Item = Cow<'_, str>> {
        glob::glob(
            &self
                .base_path()
                .join(format!("*.{}", Self::EXT))
                .display()
                .to_string(),
        )
        .unwrap()
        .filter_map(|p| p.ok())
        .filter_map(|p| p.file_stem().map(|stem| stem.display().to_string()))
        .map(Cow::Owned)
    }

    fn get_transient(&self, key: &str) -> Option<T> {
        self.view_cache(|cache| cache.get(key).cloned())
    }

    fn put_cache(&self, key: &str, value: T) {
        self.update_cache(|cache| cache.update(key.into(), value.clone()))
    }

    fn remove(&self, key: &str) -> anyhow::Result<()> {
        self.update_cache(|cache| cache.without(key));
        let path = self.base_path().join(key).with_extension(Self::EXT);
        std::fs::remove_file(path)?;

        Ok(())
    }

    fn rename(&self, old_name: &str, new_name: &str) -> anyhow::Result<()> {
        self.update_cache(|cache| {
            if let Some(value) = cache.get(old_name) {
                cache
                    .without(old_name)
                    .update(new_name.to_string(), value.clone())
            } else {
                cache.as_ref().clone()
            }
        });

        let old_path = self.base_path().join(old_name).with_extension(Self::EXT);
        let new_path = self.base_path().join(new_name).with_extension(Self::EXT);
        if old_path.exists() {
            std::fs::rename(old_path, new_path)?;
        }

        Ok(())
    }

    fn load(&self, name: &str) -> anyhow::Result<T> {
        if let Some(value) = self.view_cache(|cache| cache.get(name).cloned()) {
            return Ok(value);
        }

        let path = self.base_path().join(name).with_extension(Self::EXT);
        let file = OpenOptions::new().read(true).open(path)?;

        let value: T = serde_yaml_ng::from_reader(file)?;
        self.put_cache(name, value.clone());

        Ok(value)
    }

    /// Loads every entry from disk into the cache, skipping any broken files
    fn preload_all(&self) {
        let names: Vec<_> = self.names().map(|n| n.into_owned()).collect();

        for name in names {
            if let Err(err) = self.load(&name) {
                tracing::error!("Could not load {name}: {err:?}");
            }
        }
    }

    fn save(&self, name: &str, value: T) -> anyhow::Result<()> {
        if !name.is_empty() {
            self.put_cache(name, value.clone());

            let path = self.base_path().join(name).with_extension(Self::EXT);
            let writer = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)?;

            serde_yaml_ng::to_writer(writer, &value)?;
        }

        Ok(())
    }

    fn update(&self, name: &str, cb: impl Fn(T) -> T) -> anyhow::Result<()> {
        let item = self.load(name)?;
        self.save(name, cb(item))?;
        Ok(())
    }
}
