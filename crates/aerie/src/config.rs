use arc_swap::ArcSwap;
use cached::proc_macro::cached;
use delegate::delegate;
use glob::{Pattern, PatternError};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use crate::rmcp::model::Tool;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use clap::{Parser, Subcommand};

#[derive(Parser, Clone, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// An ephemeral file handle to dotenv formatted secrets
    #[arg(long, short)]
    pub env: Option<PathBuf>,

    #[arg(long, short)]
    pub session: Option<String>,

    #[arg(long, short)]
    pub config: Option<PathBuf>,

    #[arg(long)]
    pub session_dir: Option<PathBuf>,

    #[arg(long)]
    pub workflow_dir: Option<PathBuf>,

    #[arg(long)]
    pub tool_dir: Option<PathBuf>,

    #[arg(long)]
    pub backup_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Clone, Debug)]
pub enum Command {
    Session {
        #[command(subcommand)]
        subcmd: SessionCommand,
    },
}

#[derive(Subcommand, Clone, Debug)]
pub enum SessionCommand {
    List,
}

#[inline]
fn is_zero(x: &u64) -> bool {
    *x == 0
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct SeedConfig {
    pub value: Arc<AtomicU64>,

    #[serde(default, skip_serializing_if = "is_zero")]
    pub increment: u64,
}

impl PartialEq for SeedConfig {
    fn eq(&self, other: &Self) -> bool {
        let a = self.value.load(Ordering::Relaxed);
        let b = other.value.load(Ordering::Relaxed);
        a == b && self.increment == other.increment
    }
}

impl Eq for SeedConfig {}

#[non_exhaustive]
#[derive(
    Default,
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumIter,
    strum::EnumMessage,
    strum::EnumString,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ModelRole {
    #[default]
    #[strum(message = "Fallback model when using an unassigned role")]
    Default,

    #[strum(message = "Small model with low latency and high throughput")]
    Fast,

    #[strum(message = "A large general-purpose model for complex tasks")]
    Large,

    #[strum(message = "A model that thinks/reasons before replying")]
    Reasoning,

    #[strum(message = "Model that excels at creative writing")]
    Creative,

    #[strum(message = "Model for code generation and review")]
    Programming,

    #[strum(message = "Model specializing in structured outputs and tool calls")]
    Structured,

    #[strum(message = "A long context model for summarizing large documents")]
    Condenser,

    #[strum(message = "A visual language model that reads both texts and images")]
    Vision,

    #[strum(message = "Literal model name. Can only be passed via node input.")]
    #[strum(default, transparent)]
    #[serde(untagged)]
    Custom(String),
}

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone)]
pub struct RoleEntry {
    pub name: String,
    pub roles: im::OrdSet<ModelRole>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct ProfileMap(im::OrdMap<String, im::Vector<RoleEntry>>);

impl Default for ProfileMap {
    fn default() -> Self {
        Self(im::ordmap!(
                "default".into() => im::vector![Default::default()]
        ))
    }
}

impl ProfileMap {
    delegate! {
        to self.0 {
            pub fn is_empty(&self) -> bool;
            pub fn keys(&self) -> impl Iterator<Item=&String>;
            pub fn contains_key(&self, key: &str) -> bool;
            pub fn get(&self, key: &str) -> Option<&im::Vector<RoleEntry>>;
            pub fn insert(&mut self, key: String, value: im::Vector<RoleEntry>) -> Option<im::Vector<RoleEntry>>;
            pub fn remove(&mut self, key: &str) -> Option<im::Vector<RoleEntry>>;
        }
    }

    pub fn first_key(&self) -> Option<String> {
        self.0.keys().next().cloned()
    }

    pub fn get_or_create(&mut self, key: String) -> &mut im::Vector<RoleEntry> {
        self.0.entry(key).or_default()
    }
}

/// Persistent settings managed exclusively from the UI thread.
///
/// User configured preferences are stored in `Settings` instead.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone)]
pub struct ConfigState {
    /// Models previously used in the UI
    #[serde(default, skip_serializing_if = "im::Vector::is_empty")]
    pub prev_models: im::Vector<String>,

    /// Name of the active workflow
    #[serde(default)]
    pub workflow: String,

    #[serde(default)]
    pub session: String,

    /// Directory to export workflows
    #[serde(default)]
    pub export_dir: PathBuf,

    /// Directory to save outputs to
    #[serde(default)]
    pub output_dir: PathBuf,
}

/// Manages synchronizing state between memory and disk
pub struct ConfigStateStore {
    // Currently, facilitates UI repainting
    modtime: Instant,

    /// The backing disk store
    db: sled::Db,

    /// The live in-memory state
    state: ConfigState,
}

impl Deref for ConfigStateStore {
    type Target = ConfigState;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl ConfigStateStore {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut this = Self {
            modtime: Instant::now(),
            db: sled::open(path)?,
            state: Default::default(),
        };

        this.refresh()?;

        Ok(this)
    }

    pub fn modtime(&self) -> Instant {
        self.modtime
    }

    /// Load settings from disk
    pub fn refresh(&mut self) -> anyhow::Result<&ConfigState> {
        if let Some(bytes) = self.db.get(b"prev_models")? {
            self.state.prev_models = postcard::from_bytes(&bytes)?;
        }

        if let Some(bytes) = self.db.get(b"workflow")? {
            self.state.workflow = postcard::from_bytes(&bytes)?;
        }

        if let Some(bytes) = self.db.get(b"session")? {
            self.state.session = postcard::from_bytes(&bytes)?;
        }

        if let Some(bytes) = self.db.get(b"export_dir")? {
            self.state.export_dir = postcard::from_bytes(&bytes)?;
        }

        if let Some(bytes) = self.db.get(b"output_dir")? {
            self.state.output_dir = postcard::from_bytes(&bytes)?;
        }

        self.modtime = Instant::now();

        Ok(&self.state)
    }

    /// Runs the callback and writes any state changes to disk
    pub fn update<T>(&mut self, cb: impl FnOnce(&mut ConfigState) -> T) -> anyhow::Result<T> {
        let baseline = self.state.clone();
        let result = cb(&mut self.state);

        if baseline != self.state {
            self.modtime = Instant::now();

            if baseline.prev_models != self.state.prev_models {
                let bytes = postcard::to_allocvec(&self.state.prev_models)?;
                self.db.insert(b"prev_models", bytes)?;
            }

            if baseline.workflow != self.state.workflow {
                let bytes = postcard::to_allocvec(&self.state.workflow)?;
                self.db.insert(b"workflow", bytes)?;
            }

            if baseline.session != self.state.session {
                let bytes = postcard::to_allocvec(&self.state.session)?;
                self.db.insert(b"session", bytes)?;
            }

            if baseline.export_dir != self.state.export_dir {
                let bytes = postcard::to_allocvec(&self.state.export_dir)?;
                self.db.insert(b"export_dir", bytes)?;
            }

            if baseline.output_dir != self.state.output_dir {
                let bytes = postcard::to_allocvec(&self.state.output_dir)?;
                self.db.insert(b"output_dir", bytes)?;
            }
        }

        Ok(result)
    }

    pub fn set_output_dir(&mut self, target: Option<impl AsRef<Path>>) {
        let _ = self.update(|state| {
            if let Some(path) = target.map(|p| p.as_ref().to_path_buf()) {
                state.output_dir = path;
            }
        });
    }

    pub fn set_export_dir(&mut self, target: Option<impl AsRef<Path>>) {
        let _ = self.update(|state| {
            if let Some(path) = target.map(|p| p.as_ref().to_path_buf()) {
                state.export_dir = path;
            }
        });
    }

    pub fn set_session(&mut self, value: impl Into<String>) {
        let _ = self.update(|state| {
            state.session = value.into();
        });
    }
}

/// User managed preferences via file or settings tile in UI.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone)]
pub struct Preferences {
    /// Profiles containing models tagged with roles
    #[serde(default, skip_serializing_if = "ProfileMap::is_empty")]
    pub models: ProfileMap,

    /// The active model profile
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile: String,

    #[serde(default)]
    pub temperature: f64,

    // Timeout on total request or non-streaming probably not useful, given
    // variability of payload sizes.
    //
    /// Timeout between streaming updates.
    ///
    /// note: non-streaming timeouts controlled by underlying HTTP client.
    #[serde(default)]
    pub stream_idle: Option<u64>,

    /// PRNG seed for completion providers (which typically ignore it)
    pub seed: Option<SeedConfig>,

    #[serde(default)]
    pub autoscroll: bool,

    /// Whether to rerun only changed/selected nodes or all dependents
    #[serde(default)]
    pub cascade: bool,

    #[serde(default)]
    pub autoruns: usize,

    // Making this configurable since not 100% confident in the streaming implementation
    // The runner will fall back to non-streaming.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub streaming: bool,

    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub autosave: bool,

    /// Max linear dimension to downscale images sent to LLM
    #[serde(default)]
    pub image_size: Option<u32>,

    // Don't clobber unknown settings
    #[serde(flatten)]
    pub _extra: im::OrdMap<String, serde_json::Value>,
}

impl Preferences {
    pub fn has_profile(&self, name: &str) -> bool {
        self.models.contains_key(name)
    }

    pub fn get_model_map(&self) -> BTreeMap<ModelRole, String> {
        let profile = if self.profile.is_empty() {
            "default"
        } else {
            self.profile.as_str()
        };

        if let Some(models) = self.models.get(profile) {
            models
                .iter()
                .flat_map(|entry| entry.roles.iter().map(|r| (r.clone(), entry.name.clone())))
                .collect()
        } else {
            Default::default()
        }
    }
}

pub trait ConfigExt {
    fn view<T>(&self, cb: impl FnMut(&Preferences) -> T) -> T;

    fn update<T>(&self, cb: impl FnOnce(&mut Preferences) -> T) -> T;
}

impl ConfigExt for Arc<RwLock<Preferences>> {
    fn view<T>(&self, mut cb: impl FnMut(&Preferences) -> T) -> T {
        let settings = self.read().unwrap();
        cb(&settings)
    }

    // TODO: handle auto-save
    fn update<T>(&self, cb: impl FnOnce(&mut Preferences) -> T) -> T {
        let mut settings = self.write().unwrap();
        cb(&mut settings)
    }
}

// Ideally have a macro take the arc, a field and callback.
// Only clones if callback returns different value for field.
impl ConfigExt for Arc<ArcSwap<Preferences>> {
    fn view<T>(&self, mut cb: impl FnMut(&Preferences) -> T) -> T {
        let settings = self.load();
        cb(&settings)
    }

    // Clone to stack for working copy. Not sure which way is better
    // This compares every field
    fn update<T>(&self, cb: impl FnOnce(&mut Preferences) -> T) -> T {
        let mut settings = self.load().as_ref().clone();

        let result = cb(&mut settings);

        // Only move to stack if changed
        if settings != *self.load().as_ref() {
            self.store(Arc::new(settings));
        }

        result
    }

    // This also clones the whole object (usually), but to the heap
    // fn update<T>(&self, cb: impl FnOnce(&mut Settings) -> T) -> T {
    //     let mut settings = self.load_full();
    //
    //     // This is going to constantly clone the object.
    //     // We should optimize to only clone on actual changes
    //     let res = cb(Arc::make_mut(&mut settings));
    //
    //     self.store(settings);
    //
    //     res
    // }
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone)]
pub struct ToolSettings {
    #[serde(default, skip_serializing_if = "im::OrdMap::is_empty")]
    pub provider: im::OrdMap<String, ToolSpec>,

    #[serde(default, skip_serializing_if = "im::OrdMap::is_empty")]
    pub toolset: im::OrdMap<String, ToolSelector>,
}

/// Configuration to access a tool provider
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(tag = "type")]
pub enum ToolSpec {
    Stdio {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        enabled: bool,

        #[serde(default)]
        preface: Option<String>,

        #[serde(default)]
        dir: Option<PathBuf>,

        #[serde(default, skip_serializing_if = "String::is_empty")]
        env: String,

        command: String,

        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,

        /// Timeout in seconds
        #[serde(default)]
        timeout: Option<u64>,
    },
    HTTP {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        enabled: bool,

        #[serde(default)]
        preface: Option<String>,

        uri: String,

        /// : environment var for API key
        auth_var: Option<String>,

        /// Timeout in seconds
        #[serde(default)]
        timeout: Option<u64>,
    },
}

impl Default for ToolSpec {
    fn default() -> Self {
        ToolSpec::Stdio {
            enabled: false,
            preface: None,
            dir: None,
            env: Default::default(),
            command: String::new(),
            args: Vec::new(),
            timeout: None,
        }
    }
}

impl ToolSpec {
    pub fn enabled(&self) -> bool {
        match self {
            ToolSpec::Stdio { enabled, .. } => *enabled,
            ToolSpec::HTTP { enabled, .. } => *enabled,
        }
    }

    pub fn set_enabled(&mut self, value: bool) {
        match self {
            ToolSpec::Stdio { enabled, .. } => *enabled = value,
            ToolSpec::HTTP { enabled, .. } => *enabled = value,
        }
    }

    pub fn preface(&self) -> Option<&str> {
        match self {
            ToolSpec::Stdio { preface, .. } => preface.as_deref(),
            ToolSpec::HTTP { preface, .. } => preface.as_deref(),
        }
    }
    pub fn timeout(&self) -> Option<u64> {
        match self {
            ToolSpec::Stdio { timeout, .. } => *timeout,
            ToolSpec::HTTP { timeout, .. } => *timeout,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Ternary<T>
where
    T: std::fmt::Debug + PartialOrd + Ord,
{
    None,
    Some(im::OrdSet<T>),
    All,
}

#[derive(Debug, Default, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSelector(pub im::OrdSet<String>);

impl ToolSelector {
    pub fn empty() -> Self {
        Self(im::OrdSet::new())
    }

    pub fn only(value: &str) -> Self {
        Self(im::ordset![value.to_string()])
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn all() -> Self {
        Self::empty().with_include("*", "*")
    }

    pub fn is_all(&self) -> bool {
        self.0.contains("*/*")
    }

    pub fn with_include(mut self, provider: &str, tool: &str) -> Self {
        self.0.insert(format!("{provider}/{tool}"));
        self
    }

    pub fn add(&mut self, selector: &str) {
        self.0.insert(selector.to_string());
    }

    pub fn remove(&mut self, selector: &str) {
        self.0.remove(selector);
    }

    pub fn include(&mut self, provider: &str, tool: &Tool) {
        self.add(&format!("{provider}/{}", tool.name));
    }

    pub fn provider_selection(&'_ self, provider: &str) -> Ternary<Cow<'_, str>> {
        if self.0.contains("*/*") || self.0.contains(&format!("{provider}/*")) {
            Ternary::All
        } else {
            let prefix = format!("{provider}/");
            let tools: im::OrdSet<_> = self
                .0
                .iter()
                .filter_map(|t| t.strip_prefix(&prefix))
                .map(Cow::Borrowed)
                .collect();

            if tools.is_empty() {
                Ternary::None
            } else {
                Ternary::Some(tools)
            }
        }
    }

    pub fn apply(&self, provider: &str, tool_name: &str) -> bool {
        self.0
            .iter()
            .filter_map(|it| tool_glob(it.clone()).ok())
            .any(|it| it.matches(&format!("{provider}/{}", tool_name)))
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|s| s.as_str())
    }
}

#[cached(result = true)]
pub fn tool_glob(pattern: String) -> Result<Pattern, PatternError> {
    Pattern::new(&pattern)
}
