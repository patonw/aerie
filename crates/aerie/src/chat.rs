use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::anyhow;
use arc_swap::ArcSwap;
use rig::message::Message;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use uuid::Uuid;

#[derive(Clone)]
pub struct ChatSession {
    pub path: Arc<Option<PathBuf>>,
    pub history: Arc<ArcSwap<ChatHistory>>, // Interior mutability without locking
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
            history: Arc::new(ArcSwap::from_pointee(history)),
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
        if self.path.is_some() {
            let history = self.history.load();
            self.save_ref(&history)
        } else {
            Ok(())
        }
    }

    pub fn view<T>(&self, mut cb: impl FnMut(&ChatHistory) -> T) -> T {
        let history = self.history.load();
        cb(&history)
    }

    // callbacks likely expensive (e.g. invoking remote LLM) so retrying not desirable.
    // Fails on concurent update rather than clobbering history via `store`
    pub fn transform<F>(&self, cb: F) -> anyhow::Result<()>
    where
        F: FnOnce(&ChatHistory) -> anyhow::Result<Cow<ChatHistory>>,
    {
        let history = self.history.load();
        if let Cow::Owned(result) = cb(&history)? {
            // self.history.store(Arc::new(result));
            let prev = self
                .history
                .compare_and_swap(history.deref(), Arc::new(result));

            if !Arc::ptr_eq(history.deref(), prev.deref()) {
                return Err(anyhow!("Conflict while updating history"));
            }

            if self.path.is_some() {
                self.save_ref(&self.history.load())?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

impl std::cmp::Eq for ChatContent {} // ???

impl std::hash::Hash for ChatContent {
    // TODO: proper implementation rig::Message is the sticking point
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
    }
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
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatEntry {
    pub id: Uuid,
    pub parent: Option<Uuid>,
    pub aside: Option<Uuid>,
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
#[derive(Debug, PartialEq, Hash, Eq, Clone, Serialize, Deserialize)]
pub struct ChatHistory {
    pub store: im::OrdMap<Uuid, ChatEntry>,

    pub branches: im::OrdMap<String, Uuid>,

    /// First message in iteration
    pub base: Option<Uuid>,

    /// Name of current branch
    pub head: String,
}

impl Default for ChatHistory {
    fn default() -> Self {
        Self {
            store: Default::default(),
            branches: Default::default(),
            base: None,
            head: "default".to_string(),
        }
    }
}

impl ChatHistory {
    pub fn switch(&'_ self, head: &str) -> Cow<'_, Self> {
        let mut result = Cow::Borrowed(self);
        if self.head != head {
            result.to_mut().head = head.to_string();
        }

        result
    }

    pub fn with_base(&'_ self, base: Option<Uuid>) -> Cow<'_, Self> {
        let mut result = Cow::Borrowed(self);
        if self.base != base {
            result.to_mut().base = base;
        }

        result
    }

    pub fn push(
        &'_ self,
        content: ChatContent,
        branch: Option<impl AsRef<str>>,
    ) -> anyhow::Result<Cow<'_, Self>> {
        self.extend(std::iter::once(content), branch)
    }

    pub fn extend(
        &'_ self,
        contents: impl std::iter::IntoIterator<Item = ChatContent>,
        branch: Option<impl AsRef<str>>,
    ) -> anyhow::Result<Cow<'_, Self>> {
        let mut result = Cow::Borrowed(self);

        for content in contents {
            let (branch, parent) = if let Some(branch) = branch.as_ref() {
                (
                    branch.as_ref().to_string(),
                    result.branches.get(branch.as_ref()).cloned(),
                )
            } else {
                result.head_branch()
            };

            let id = Uuid::new_v4();
            let entry = ChatEntry {
                id,
                parent,
                aside: None,
                content,
                branch: branch.clone(),
            };

            result.to_mut().store = result.store.update(id, entry);
            result.to_mut().branches = result.branches.update(branch.clone(), id);
        }

        Ok(result)
    }

    pub fn aside(
        &'_ self,
        contents: impl std::iter::IntoIterator<Item = ChatContent>,
    ) -> anyhow::Result<Cow<'_, Self>> {
        let mut result = Cow::Borrowed(self);
        // let mut contents = contents.into_iter();
        let mut first: Option<Uuid> = None;
        let mut last: Option<Uuid> = None;

        for content in contents {
            let id = Uuid::new_v4();
            let entry = ChatEntry {
                id,
                parent: last,
                aside: None,
                content,
                branch: "".to_string(),
            };

            result.to_mut().store = result.store.update(id, entry);

            if first.is_none() {
                first = Some(id);
            }
            last = Some(id);
        }

        // Nothing to add
        if first.is_none() {
            return Ok(result);
        }

        let (branch, parent) = result.head_branch();

        let first = first
            .and_then(|id| result.to_mut().store.get_mut(&id))
            .unwrap();

        let first_id = first.id;
        first.branch = branch.clone();
        first.parent = parent;

        let last = last
            .and_then(|id| result.to_mut().store.get_mut(&id))
            .unwrap();

        let last_id = last.id;
        last.aside = last.parent;
        last.parent = Some(first_id);
        last.branch = branch.clone();

        result.to_mut().branches = result.branches.update(self.head.clone(), last_id);

        Ok(result)
    }

    pub fn has_branch(&self, name: &str) -> bool {
        self.branches.contains_key(name)
    }

    fn head_branch(&self) -> (String, Option<Uuid>) {
        let branch = self.head.clone();
        let uuid = self.branches.get(&branch).cloned();
        (branch, uuid)
    }

    pub fn create_branch(
        &'_ self,
        branch: &str,
        parent: Option<Uuid>,
    ) -> anyhow::Result<Cow<'_, Self>> {
        let mut result = Cow::Borrowed(self);
        // Nah, a little pointless to branch from current head
        // let parent = parent.or_else(|| self.head_branch().1);

        // If no parent, then creates a new root
        if !self.branches.contains_key(branch)
            && let Some(parent) = parent
        {
            result.to_mut().branches = self.branches.update(branch.to_string(), parent);
        }

        // TODO: dedup `switch` - How to chain ops on cows?
        if self.head != branch {
            result.to_mut().head = branch.to_string();
        }

        Ok(result)
    }

    pub fn find_parent(&self, id: Uuid) -> Option<Uuid> {
        self.store.get(&id).and_then(|it| it.parent)
    }

    pub fn last(&self) -> Option<&ChatEntry> {
        self.branches
            .get(&self.head)
            .and_then(|it| self.store.get(it))
    }

    pub fn iter(&self) -> impl Iterator<Item = &ChatEntry> {
        self.branches
            .get(&self.head)
            .into_iter()
            .flat_map(|end_msg| self.iter_between(None, *end_msg))
    }

    pub fn rev_iter(&self) -> impl Iterator<Item = &ChatEntry> {
        self.branches
            .get(&self.head)
            .into_iter()
            .flat_map(|end_msg| ChatRevIter(self, Some(*end_msg)))
    }

    pub fn iter_aside(&self, entry: &ChatEntry) -> impl Iterator<Item = &ChatEntry> {
        entry.aside.iter().flat_map(|end| {
            let start = entry.parent;

            self.iter_between(start, *end)
        })
    }

    /// Returns an iterator of messages starting after start_msg and ending with end_msg.
    /// If no start_msg is set, iterator begins at the root.
    pub fn iter_between(
        &self,
        start_msg: Option<Uuid>,
        end_msg: Uuid,
    ) -> impl Iterator<Item = &ChatEntry> {
        let mut buffer: Vec<&ChatEntry> = Vec::new();
        let mut cursor = Some(end_msg);
        let start_msg = start_msg.or(self.base);

        while let Some(id) = cursor {
            if let Some(entry) = self.store.get(&id) {
                if Some(id) == start_msg {
                    break;
                }
                buffer.push(entry);
                cursor = entry.parent;
            }
        }

        buffer.reverse();
        buffer.into_iter()
    }

    pub fn iter_msgs(&self) -> impl Iterator<Item = &Message> {
        self.iter().filter_map(|entry| match &entry.content {
            ChatContent::Message(message) => Some(message),
            _ => None,
        })
    }

    pub fn lineage(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut buffer: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

        // Add relations between non-empty branches
        for entry in self.store.values() {
            let parent_branch: Option<&str> = entry
                .parent
                .as_ref()
                .and_then(|it| self.store.get(it))
                .map(|it| it.branch.as_ref());

            if entry.branch.is_empty() {
                // Ignore branchless entries from side conversations
                continue;
            } else if let Some(pb) = parent_branch {
                if pb != entry.branch && !pb.is_empty() {
                    buffer
                        .entry(pb.to_string())
                        .or_default()
                        .insert(entry.branch.clone());
                }
            } else {
                // It's a root entry
                buffer
                    .entry("".into())
                    .or_default()
                    .insert(entry.branch.clone());
            }
        }

        // Account for empty branches
        for (branch, id) in &self.branches {
            let parent = self.store.get(id).map(|it| it.branch.clone());

            if !branch.is_empty() && parent != Some(branch.clone()) {
                buffer
                    .entry(parent.unwrap_or_default())
                    .or_default()
                    .insert(branch.clone());
            }
        }

        // And a new head
        if !self.head.is_empty() {
            let parent = self
                .branches
                .get(&self.head)
                .and_then(|id| self.store.get(id))
                .map(|it| it.branch.clone());

            if parent != Some(self.head.clone()) {
                buffer
                    .entry(parent.unwrap_or_default())
                    .or_default()
                    .insert(self.head.clone());
            }
        }

        buffer
    }

    pub fn rename_branch(&'_ self, branch: &str, new_name: &str) -> anyhow::Result<Cow<'_, Self>> {
        let mut result = Cow::Borrowed(self);

        let Some(head_id) = self.branches.get(branch).cloned() else {
            return Err(anyhow!("Branch does not exist"));
        };

        result.to_mut().branches = self
            .branches
            .without(branch)
            .update(new_name.to_string(), head_id);

        if self.head == branch {
            result.to_mut().head = new_name.to_string();
        }

        let mut cursor = head_id;
        loop {
            let Some(mut node) = result.store.get(&cursor).cloned() else {
                break;
            };

            if node.branch != branch {
                break;
            }

            node.branch = new_name.to_string();

            let parent = node.parent;
            result.to_mut().store = result.store.update(cursor, node);

            if let Some(parent) = &parent {
                cursor = *parent;
            } else {
                break;
            }
        }

        Ok(result)
    }

    // TODO: buggy - promoting second level flattens hierarchy (non-destructive though)
    pub fn promote_branch(&'_ self, branch: &str) -> anyhow::Result<Cow<'_, Self>> {
        let mut result = Cow::Borrowed(self);
        let Some(head_id) = result.branches.get(branch) else {
            return Ok(result);
        };

        let mut cursor = *head_id;
        let mut ancestor: Option<String> = None;

        // crawl up tree until first ancestor. Rename until different ancestor.
        loop {
            let Some(mut node) = result.store.get(&cursor).cloned() else {
                break;
            };

            if let Some(target) = &ancestor {
                if &node.branch != target {
                    break;
                }

                node.branch = branch.to_string();
            } else if node.branch != branch {
                ancestor = Some(node.branch.clone());
                node.branch = branch.to_string();
            }

            let parent = node.parent;
            result.to_mut().store = result.store.update(cursor, node);

            if let Some(parent) = parent {
                cursor = parent;
            } else {
                break;
            }
        }

        Ok(result)
    }

    pub fn prune_branch(&'_ self, branch: &str) -> anyhow::Result<Cow<'_, Self>> {
        let Some(head_id) = self.branches.get(branch).cloned() else {
            return Err(anyhow!("Cannot prune current branch"));
        };

        let mut result = self.clone();

        result.branches = self.branches.without(branch);

        let mut cursor = head_id;
        loop {
            let Some(node) = result.store.get(&cursor) else {
                break;
            };

            if !node.branch.is_empty() && node.branch != branch {
                if result.head == branch {
                    result.head = node.branch.clone();
                }
                break;
            }

            let parent = node.aside.or(node.parent);
            result.store = result.store.without(&cursor);

            if let Some(parent) = parent {
                cursor = parent;
            } else {
                break;
            }
        }

        Ok(if self == &result {
            Cow::Borrowed(self)
        } else {
            Cow::Owned(result)
        })
    }

    /// Returns the last message common in both histories. If one is a strict extension of the
    /// other, then the common id will be the head of the base history.
    pub fn find_common(&self, other: &Self) -> Option<Uuid> {
        let mut seen: BTreeSet<Uuid> = Default::default();

        let mut id_a = self.branches.get(&self.head).cloned();
        let mut id_b = other.branches.get(&other.head).cloned();

        while id_a.is_some() || id_b.is_some() {
            if let Some(id) = id_a {
                if seen.contains(&id) {
                    return Some(id);
                }
                seen.insert(id);
            }

            if let Some(id) = id_b {
                if seen.contains(&id) {
                    return Some(id);
                }
                seen.insert(id);
            }

            id_a = id_a
                .and_then(|id| self.store.get(&id))
                .and_then(|ent| ent.parent);

            id_b = id_b
                .and_then(|id| other.store.get(&id))
                .and_then(|ent| ent.parent);
        }

        None
    }
}

struct ChatRevIter<'a>(&'a ChatHistory, Option<Uuid>);

impl<'a> Iterator for ChatRevIter<'a> {
    type Item = &'a ChatEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(id) = &self.1
            && let Some(entry) = self.0.store.get(id)
            && Some(entry.id) != self.0.base
        {
            self.1 = entry.parent;
            Some(entry)
        } else {
            None
        }
    }
}
