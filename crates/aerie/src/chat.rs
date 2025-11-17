use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use anyhow::anyhow;
use rig::message::Message;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use uuid::Uuid;

#[derive(Clone)]
pub struct ChatSession {
    pub path: Arc<Option<PathBuf>>,
    pub history: Arc<RwLock<ChatHistory>>,
}

impl ChatSession {
    pub fn load(path: Option<impl AsRef<Path>>) -> Self {
        let history = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_yml::from_str::<ChatHistory>(&s).ok())
            .unwrap_or_default();

        Self {
            path: Arc::new(path.map(|p| p.as_ref().to_path_buf())),
            history: Arc::new(RwLock::new(history)),
        }
    }

    fn save_ref(&self, history: &ChatHistory) -> anyhow::Result<()> {
        let path = self
            .path
            .as_deref()
            .ok_or_else(|| anyhow!("No session path set"))?;

        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        serde_yml::to_writer(file, history)?;

        Ok(())
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let history_r = self.history.read().unwrap();
        self.save_ref(&history_r)
    }

    pub fn view<T>(&self, mut cb: impl FnMut(&ChatHistory) -> T) -> T {
        let history_r = self.history.read().unwrap();
        cb(&history_r)
    }

    pub fn update<T>(&self, cb: impl FnOnce(&mut ChatHistory) -> T) -> anyhow::Result<T> {
        let span = tracing::info_span!("Updating session");
        span.in_scope(|| {
            let mut history_rw = self.history.write().unwrap();
            let res = cb(&mut history_rw);

            if self.path.is_some() {
                self.save_ref(&history_rw)?;
            }
            Ok(res)
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ChatContent {
    Message(Message),
    Aside {
        automation: String,
        prompt: String,
        collapsed: bool,
        content: Vec<Message>,
    },
    Error(String),
}

impl From<Result<Message, String>> for ChatContent {
    fn from(value: Result<Message, String>) -> Self {
        match value {
            Ok(msg) => ChatContent::Message(msg),
            Err(err) => ChatContent::Error(err),
        }
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEntry {
    pub id: Uuid,
    pub parent: Option<Uuid>,
    pub branch: String,
    pub content: ChatContent,
}

impl Deref for ChatEntry {
    type Target = ChatContent;

    fn deref(&self) -> &Self::Target {
        &self.content
    }
}

#[skip_serializing_none]
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ChatHistory {
    pub store: BTreeMap<Uuid, ChatEntry>,
    pub branches: BTreeMap<String, Uuid>,
    pub head: Option<String>,
}

impl ChatHistory {
    pub fn push(&mut self, content: ChatContent, branch: Option<impl AsRef<str>>) {
        let (branch, parent) = if let Some(branch) = branch {
            (
                branch.as_ref().to_string(),
                self.branches.get(branch.as_ref()).cloned(),
            )
        } else {
            self.head_branch()
        };

        let id = Uuid::new_v4();
        let entry = ChatEntry {
            id,
            parent,
            content,
            branch: branch.clone(),
        };
        self.store.insert(id, entry);

        self.branches.insert(branch.clone(), id);
    }

    pub fn extend(
        &mut self,
        contents: impl std::iter::IntoIterator<Item = ChatContent>,
        branch: Option<impl AsRef<str>>,
    ) {
        for content in contents {
            self.push(content, branch.as_ref());
        }
    }

    pub fn clear(&mut self) {
        self.store.clear();
        self.branches.clear();
        self.head = None;
    }

    fn head_branch(&mut self) -> (String, Option<Uuid>) {
        if self.head.is_none() {
            self.head = Some("default".into());
        }

        let branch = self.head.to_owned().unwrap();
        let uuid = self.branches.get(&branch).cloned();
        (branch, uuid)
    }

    pub fn switch_branch(&mut self, branch: &str, parent: Option<Uuid>) {
        // Nah, a little pointless to branch from current head
        // let parent = parent.or_else(|| self.head_branch().1);

        // If no parent, then creates a new root
        if !self.branches.contains_key(branch)
            && let Some(parent) = parent
        {
            self.branches.insert(branch.to_string(), parent);
        }

        self.head = Some(branch.into());
    }

    pub fn find_parent(&self, id: Uuid) -> Option<Uuid> {
        self.store.get(&id).and_then(|it| it.parent)
    }

    pub fn last(&self) -> Option<&ChatEntry> {
        self.head
            .as_ref()
            .and_then(|it| self.branches.get(it))
            .and_then(|it| self.store.get(it))
    }

    pub fn iter(&self) -> impl Iterator<Item = &ChatEntry> {
        self.head
            .iter()
            .flat_map(|it| self.branches.get(it))
            .flat_map(|end_msg| self.iter_between(None, *end_msg))
    }

    pub fn iter_between(
        &self,
        start_msg: Option<Uuid>,
        end_msg: Uuid,
    ) -> impl Iterator<Item = &ChatEntry> {
        let mut buffer: Vec<&ChatEntry> = Vec::new();
        let mut cursor = Some(end_msg);

        while let Some(id) = cursor {
            if let Some(entry) = self.store.get(&id) {
                buffer.push(entry);
                cursor = entry.parent;
                if Some(id) == start_msg {
                    break;
                }
            }
        }

        buffer.reverse();
        buffer.into_iter()
    }

    pub fn lineage(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut buffer: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for entry in self.store.values() {
            let parent_branch = entry
                .parent
                .as_ref()
                .and_then(|it| self.store.get(it))
                .map(|it| it.branch.as_ref());

            if let Some(pb) = parent_branch {
                if pb != entry.branch {
                    buffer
                        .entry(pb.to_string())
                        .or_default()
                        .insert(entry.branch.clone());
                }
            } else {
                // Is a root entry
                buffer
                    .entry("".into())
                    .or_default()
                    .insert(entry.branch.clone());
            }
        }

        let parent = self
            .head
            .as_ref()
            .and_then(|h| self.branches.get(h))
            .and_then(|id| self.store.get(id))
            .map(|it| it.branch.clone());

        if let Some(head) = self.head.clone()
            && parent != self.head
        {
            buffer
                .entry(parent.unwrap_or_default())
                .or_default()
                .insert(head);
        }

        buffer
    }

    pub fn rename_branch(&mut self, branch: &str, new_name: &str) -> Result<(), String> {
        let Some(head_id) = self.branches.remove(branch) else {
            return Err("Branch does not exist".into());
        };

        self.branches.insert(new_name.to_string(), head_id);
        if self.head.as_deref() == Some(branch) {
            self.head = Some(new_name.to_string());
        }

        let mut cursor = head_id;
        loop {
            let Some(node) = self.store.get_mut(&cursor) else {
                break;
            };

            if node.branch != branch {
                break;
            }

            node.branch = new_name.to_string();

            if let Some(parent) = &node.parent {
                cursor = *parent;
            } else {
                break;
            }
        }

        Ok(())
    }

    pub fn promote_branch(&mut self, branch: &str) {
        let Some(head_id) = self.branches.get(branch) else {
            return;
        };

        let mut cursor = *head_id;
        let mut ancestor: Option<String> = None;

        // crawl up tree until first ancestor. Rename until different ancestor.
        loop {
            let Some(node) = self.store.get_mut(&cursor) else {
                break;
            };

            dbg!((&cursor, &ancestor, &node));

            if let Some(target) = &ancestor {
                if &node.branch != target {
                    break;
                }

                node.branch = branch.to_string();
            } else if node.branch != branch {
                ancestor = Some(node.branch.clone());
                node.branch = branch.to_string();
            }

            if let Some(parent) = &node.parent {
                cursor = *parent;
            } else {
                break;
            }
        }
    }

    pub fn prune_branch(&mut self, branch: &str) {
        let Some(head_id) = self.branches.remove(branch) else {
            return;
        };
        let mut cursor = head_id;
        loop {
            let Some(node) = self.store.get(&cursor) else {
                break;
            };

            if node.branch != branch {
                if self.head.as_deref() == Some(branch) {
                    self.head = Some(node.branch.clone());
                }
                break;
            }

            let node = self.store.remove(&cursor).unwrap();

            if let Some(parent) = &node.parent {
                cursor = *parent;
            } else {
                break;
            }
        }
    }
}
