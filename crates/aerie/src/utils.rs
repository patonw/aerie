use std::{
    borrow::Cow,
    cmp::{Eq, PartialEq},
    collections::BinaryHeap,
    hash::Hash,
    sync::Arc,
};

use arc_swap::ArcSwap;
use decorum::E32;
use egui::mutex::Mutex;
use itertools::{Itertools, iproduct};
use rig::message::{AssistantContent, Message, ToolResultContent, UserContent};
use rpds::{List, ListSync};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct EVec2 {
    pub x: E32,
    pub y: E32,
}

impl From<egui::Vec2> for EVec2 {
    fn from(value: egui::Vec2) -> Self {
        Self {
            x: E32::assert(value.x),
            y: E32::assert(value.y),
        }
    }
}

impl From<EVec2> for egui::Vec2 {
    fn from(value: EVec2) -> Self {
        Self {
            x: value.x.into_inner(),
            y: value.y.into_inner(),
        }
    }
}

#[derive(Clone)]
pub struct AtomicBuffer<T>(pub Arc<ArcSwap<im::Vector<Arc<ArcSwap<T>>>>>);

impl<T> std::default::Default for AtomicBuffer<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<T> AtomicBuffer<T> {
    pub fn push_back(&self, content: T) -> Arc<ArcSwap<T>> {
        let cell = Arc::new(ArcSwap::from_pointee(content));
        self.0.rcu(|v| {
            let mut v = v.clone();
            Arc::make_mut(&mut v).push_back(cell.clone());
            v
        });

        cell
    }

    pub fn pop_back(&self) -> Option<Arc<T>> {
        let mut rv = None;
        self.0.rcu(|v| {
            let mut v = v.clone();
            rv = Arc::make_mut(&mut v).pop_back();
            v
        });

        rv.map(|v| v.load_full())
    }

    pub fn clear(&self) {
        self.0.store(Default::default());
    }

    delegate::delegate! {
        to self.0 {

            pub fn load(&self) -> arc_swap::Guard<Arc<im::Vector<Arc<ArcSwap<T>>>>>;
        }
    }
}

pub struct PriorityQueue<T: Ord>(Mutex<(u64, BinaryHeap<(T, u64)>)>);

impl<T: Ord> Default for PriorityQueue<T> {
    fn default() -> Self {
        Self(Mutex::new((u64::MAX, BinaryHeap::new())))
    }
}

impl<T: Ord> PriorityQueue<T> {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn insert(&self, value: T) {
        let mut data = self.0.lock();
        let count = data.0;
        data.1.push((value, count));
        data.0 -= 1;
    }

    pub fn pop(&self) -> Option<T> {
        let mut data = self.0.lock();
        data.1.pop().map(|(t, _)| t)
    }

    /// Keep only the items matching the predicate. Returns the remainder in heap order
    pub fn retain(&self, f: impl Fn(&T) -> bool) -> Vec<T> {
        let mut remainder = vec![];
        let mut target = BinaryHeap::new();
        let mut data = self.0.lock();
        while let Some((item, i)) = data.1.pop() {
            if f(&item) {
                target.push((item, i));
            } else {
                remainder.push(item);
            }
        }

        data.1 = target;

        remainder
    }

    /// Removes and returns items matching the predicate in heap order
    pub fn withdraw(&self, f: impl Fn(&T) -> bool) -> Vec<T> {
        self.retain(|item| !f(item))
    }
}

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

pub trait SwapAmend {
    type Item;

    fn amend<T>(&self, cb: impl FnOnce(&mut Self::Item) -> T) -> T;

    fn transform(
        &self,
        cb: impl FnOnce(&Self::Item) -> anyhow::Result<Self::Item>,
    ) -> anyhow::Result<()>;
}

impl<A: Clone> SwapAmend for Arc<ArcSwap<A>> {
    type Item = A;

    fn amend<T>(&self, cb: impl FnOnce(&mut Self::Item) -> T) -> T {
        let mut item = self.load_full();
        let result = cb(Arc::make_mut(&mut item));
        self.store(item);

        result
    }

    fn transform(
        &self,
        cb: impl FnOnce(&Self::Item) -> anyhow::Result<Self::Item>,
    ) -> anyhow::Result<()> {
        let item = self.load();

        let result = cb(&item)?;
        self.store(Arc::new(result));
        Ok(())
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

/// Attempts to find a JSON document surrounded by unstructured text.
/// This can require several passes over the document and should be used
/// as a last resort.
pub fn extract_json<'a, T>(input: &'a str, as_array: bool) -> Option<T>
where
    T: Deserialize<'a>,
{
    let left: Vec<_> = input
        .match_indices(if as_array { '[' } else { '{' })
        .map(|(i, _)| i)
        .collect();

    let right: Vec<_> = input
        .rmatch_indices(if as_array { ']' } else { '}' })
        .map(|(i, _)| i)
        .collect();
    let pairs = iproduct!(left, right).collect_vec();

    pairs
        .into_iter()
        .filter(|(a, b)| a < b)
        .find_map(|(a, b)| serde_json::from_str(&input[a..=b]).ok())
}
pub fn extract_json_obj<'a, T>(input: &'a str) -> Option<T>
where
    T: Deserialize<'a>,
{
    extract_json(input, false)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_no_json() {
        let input = "nothing to see here, move along";

        assert_eq!(extract_json_obj::<serde_json::Value>(input), None);
    }

    #[test]
    fn test_all_json() {
        let input = r#"{"hello": "world", "number": 1}"#;

        assert_eq!(
            extract_json_obj::<serde_json::Value>(input),
            Some(json!({"hello": "world", "number": 1}))
        );
    }

    #[test]
    fn test_some_json() {
        let input = r#"Sure. Here's{ your document: {"hello": "world", "number": 1}.\n\
        {Let me know if there's any}thing else, I can hinder you with.}."#;

        assert_eq!(
            extract_json_obj::<serde_json::Value>(input),
            Some(json!({"hello": "world", "number": 1}))
        );
    }
}
