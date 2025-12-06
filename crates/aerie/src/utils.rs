use std::{
    borrow::Cow,
    cmp::{Eq, PartialEq},
    hash::Hash,
    sync::Arc,
};

use arc_swap::ArcSwap;
use itertools::Itertools;
use rig::message::{AssistantContent, Message, ToolResultContent, UserContent};
use rpds::{List, ListSync};
use serde_json::Value;

pub trait CowExt<'a, T: Clone, E> {
    /// Flat map for cows.
    ///
    /// This is needed when composing functions that return Cow, since an intermediate call might
    /// produce an Owned while the final call returns a Borrowed to it. Since you can't return a
    /// reference to local data, a naive implementation will fail to compile. However, with this
    /// method, after the first Owned is produced, all subsequent calls will result in Owned. The
    /// only way a Borrowed can be the end result is if all calls in the chain borrow the original
    /// value.
    fn moo<F>(&self, f: F) -> Cow<'a, T>
    where
        F: FnOnce(&'_ T) -> Cow<'_, T>;

    /// Flat map for cows, bubbling up error results
    ///
    /// This is needed when composing functions that return Cow, since an intermediate call might
    /// produce an Owned while the final call returns a Borrowed to it. Since you can't return a
    /// reference to local data, a naive implementation will fail to compile. However, with this
    /// method, after the first Owned is produced, all subsequent calls will result in Owned. The
    /// only way a Borrowed can be the end result is if all calls in the chain borrow the original
    /// value.
    fn try_moo<F>(&self, f: F) -> Result<Cow<'a, T>, E>
    where
        F: FnOnce(&'_ T) -> Result<Cow<'_, T>, E>;
}

impl<'a, T: Clone, E> CowExt<'a, T, E> for Cow<'a, T> {
    fn moo<F>(&self, f: F) -> Cow<'a, T>
    where
        F: FnOnce(&'_ T) -> Cow<'_, T>,
    {
        match f(self.as_ref()) {
            Cow::Borrowed(_) => self.clone(),
            Cow::Owned(res) => Cow::Owned(res),
        }
    }

    fn try_moo<F>(&self, f: F) -> Result<Cow<'a, T>, E>
    where
        F: FnOnce(&'_ T) -> Result<Cow<'_, T>, E>,
    {
        Ok(match f(self.as_ref())? {
            Cow::Borrowed(_) => self.clone(),
            Cow::Owned(res) => Cow::Owned(res),
        })
    }
}

// Elements needs to be clonable since rcu may retry to preserve consistency.
// Hence we wrap errors in Arc
pub type ErrorList<E> = Arc<ArcSwap<ListSync<Arc<E>>>>;

pub fn new_errlist<E>() -> ErrorList<E> {
    Arc::new(ArcSwap::from_pointee(rpds::List::new_sync()))
}

/// Trait to queue non-critical errors into a central collection for later inspection
pub trait ErrorDistiller<E> {
    fn discard(&self);

    fn push(&self, err: E);

    /// Diverts errors into a sink while converting result into an option
    fn distil<T>(&self, result: Result<T, E>) -> Option<T> {
        match result {
            Ok(item) => Some(item),
            Err(err) => {
                self.push(err);
                None
            }
        }
    }
}

impl<E> ErrorDistiller<E> for ErrorList<E> {
    fn discard(&self) {
        self.store(Arc::new(List::new_sync()));
    }

    fn push(&self, err: E) {
        let err = Arc::new(err);
        self.rcu(|list| list.push_front(err.clone()));
    }
}

pub trait ImmutableSetExt<V> {
    /// Construct a new hash map by inserting a key/value mapping into a map.
    /// If the map already has a mapping for the given key and value, returns self.
    fn with(&self, v: &V) -> Self;
}

impl<V> ImmutableSetExt<V> for im::HashSet<V>
where
    V: Hash + Eq + Clone,
{
    fn with(&self, v: &V) -> Self {
        if self.contains(v) {
            self.clone()
        } else {
            self.update(v.clone())
        }
    }
}
impl<V> ImmutableSetExt<V> for im::OrdSet<V>
where
    V: Ord + Eq + Clone,
{
    fn with(&self, v: &V) -> Self {
        if self.contains(v) {
            self.clone()
        } else {
            self.update(v.clone())
        }
    }
}

pub trait ImmutableMapExt<K, V> {
    /// Construct a new hash map by inserting a key/value mapping into a map.
    /// If the map already has a mapping for the given key and value, returns self.
    fn with(&self, k: &K, v: &V) -> Self;
}

impl<K, V> ImmutableMapExt<K, V> for im::HashMap<K, V>
where
    K: Hash + Eq + Clone,
    V: PartialEq + Clone,
{
    fn with(&self, k: &K, v: &V) -> Self {
        if let Some(old_value) = self.get(k)
            && old_value == v
        {
            self.clone()
        } else {
            self.update(k.clone(), v.clone())
        }
    }
}

impl<K, V> ImmutableMapExt<K, V> for im::OrdMap<K, V>
where
    K: Ord + Eq + Clone,
    V: PartialEq + Clone,
{
    fn with(&self, k: &K, v: &V) -> Self {
        if let Some(old_value) = self.get(k)
            && old_value == v
        {
            self.clone()
        } else {
            self.update(k.clone(), v.clone())
        }
    }
}

pub fn message_party(message: &Message) -> &str {
    match message {
        Message::User { .. } => "User",
        Message::Assistant { .. } => "Assistant",
    }
}

#[derive(Debug, Clone)]
pub enum FormatOpts {
    Plain,
    Pre,
    Markdown,
    Unknown,
    Separator,
}

pub trait MessageExt {
    fn text_fmt_opts(&self) -> Vec<(String, FormatOpts)>;
}

impl MessageExt for Message {
    fn text_fmt_opts(&self) -> Vec<(String, FormatOpts)> {
        match self {
            Message::User { content } => {
                content.iter().flat_map(extract_user_content).collect_vec()
            }
            Message::Assistant { content, .. } => content
                .iter()
                .flat_map(extract_assistant_content)
                .collect_vec(),
        }
    }
}

pub fn extract_user_content(content: &UserContent) -> Vec<(String, FormatOpts)> {
    match content {
        UserContent::Text(text) => vec![(text.text.clone(), FormatOpts::Markdown)],
        UserContent::ToolResult(tool_result) => tool_result
            .content
            .iter()
            .filter_map(|m| match m {
                ToolResultContent::Text(text) => {
                    let pretty = serde_json::from_str(text.text())
                        .and_then(|v: Value| serde_json::to_string_pretty(&v))
                        .unwrap_or_else(|_| text.text().to_string());
                    Some((pretty, FormatOpts::Pre))
                }
                _ => None,
            })
            .collect_vec(),
        other => vec![(format!("{other:?}"), FormatOpts::Unknown)],
    }
}

pub fn extract_assistant_content(content: &AssistantContent) -> Vec<(String, FormatOpts)> {
    match content {
        AssistantContent::Text(text) => vec![(text.text.clone(), FormatOpts::Markdown)],
        AssistantContent::ToolCall(tool_call) => {
            let text = serde_json::to_string_pretty(&tool_call)
                .unwrap_or_else(|_| format!("{tool_call:?}"));
            vec![(text, FormatOpts::Pre)]
        }
        AssistantContent::Reasoning(reasoning) => vec![(
            reasoning.reasoning.join("\n\n---\n\n"),
            FormatOpts::Markdown,
        )],
    }
}

pub fn message_text(message: &Message) -> String {
    let Some((text, _)) = message.text_fmt_opts().into_iter().next() else {
        panic!()
    };

    text
}
