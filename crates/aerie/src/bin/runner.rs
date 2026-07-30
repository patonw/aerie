use std::{
    collections::{BTreeMap, HashMap},
    convert::identity,
    fs::OpenOptions,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    str::FromStr as _,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use aerie::{
    AgentFactory, ChatSession, Preferences,
    config::ModelRole,
    rig::message::UserContent,
    storage::CachedDirStore as _,
    toolbox::ToolStore,
    utils::{ImageResolver, message_text},
    workflow::{
        CheckContext, RootContext, RunContext, Value, Workflow,
        runner::{ExecId, RunEventCast, WorkflowRunner},
        store::WorkflowStoreDir,
        write_value,
    },
};
use anyhow::Context as _;
use arc_swap::{ArcSwap, ArcSwapOption};
use clap::{Parser, Subcommand};
use egui_snarl::Snarl;
use itertools::Itertools as _;
use lru::LruCache;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_with::skip_serializing_none;
use serde_yaml_ng as serde_yml;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};
use typed_builder::TypedBuilder;

#[skip_serializing_none]
#[derive(clap::Args, Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecArgs {
    /// The workflow file to run
    workflow: String,

    /// Initial user prompt if required by the workflow
    #[serde(default)]
    #[arg(short, long, visible_alias("prompt"))]
    input: Option<String>,

    /// Path to file containing the initial prompt
    #[serde(default)]
    #[arg(short = 'I', long)]
    input_file: Option<PathBuf>,

    /// Either a file path or data url
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[arg(long, visible_alias("image"), action = clap::ArgAction::Append)]
    images: Vec<String>,

    /// Save outputs as individual files in a directory
    #[serde(default)]
    #[arg(short, long)]
    out_dir: Option<PathBuf>,

    /// Number of extra turns to run chained workflows
    #[serde(default, skip_serializing_if = "is_zero")]
    #[arg(short, long, default_value_t = 0)]
    autoruns: usize,

    /// Prints an additional object containing the next workflow after the last run
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[arg(short = 'n', long, action = clap::ArgAction::SetTrue, default_value_t = false)]
    show_next: bool,

    /// Include workflow ids in output
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[arg(long, action = clap::ArgAction::SetTrue, default_value_t = false)]
    show_ids: bool,

    /// Pretty print console output
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[arg(short, long, action = clap::ArgAction::SetTrue, default_value_t = false)]
    pretty: bool,
}

#[inline]
fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[derive(clap::Args, Clone, Debug)]
pub struct HttpServerArgs {
    #[arg(short, long, default_value_t = String::from("localhost"))]
    host: String,

    #[arg(short, long, default_value_t = 8058)]
    port: u32,
}

#[derive(clap::Args, Clone, Debug)]
pub struct McpServerArgs {
    /// Listen with streaming HTTP transport instead of STDIO
    #[arg(long, action = clap::ArgAction::SetTrue, default_value_t = false)]
    http: bool,

    #[arg(short, long, default_value_t = 3)]
    autoruns: usize,
}

#[non_exhaustive]
#[derive(Subcommand, Clone, Debug)]
pub enum Command {
    /// Loads tool providers and checks workflows in store
    Check {
        #[arg(short, long, action = clap::ArgAction::SetTrue, default_value_t = false)]
        pretty: bool,
    },

    /// Execute a workflow and dump results to console or a directory
    Exec(ExecArgs),

    #[cfg(feature = "runner-http")]
    Serve(HttpServerArgs),

    #[cfg(feature = "runner-mcp")]
    MCP(McpServerArgs),
}

/// A minimalist workflow runner that dumps outputs to the console as a JSON object.
///
/// If you need post-processing, use external tools like jq, sed and awk.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Configuration file containing tool providers and default agent settings
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Configuration file containing tool providers and default agent settings
    #[arg(short, long)]
    profile: Option<String>,

    /// An ephemeral file handle to dotenv formatted secrets
    #[arg(long, short)]
    env: Option<PathBuf>,

    /// Directory containing workflows
    #[arg(short, long)]
    workflows: Option<PathBuf>,

    /// Directory containing tool provider definitions
    #[arg(short, long)]
    tools: Option<PathBuf>,

    /// A session to use in the workflow.
    /// Updates are discarded unless `--update` is also used.
    #[arg(short, long)]
    session: Option<PathBuf>,

    /// The session branch to use
    #[arg(short, long)]
    branch: Option<String>,

    /// Save updates to the session after running the workflow.
    #[arg(long, action)]
    update_session: bool,

    /// Model(s) to use in the workflow.
    ///
    /// Entries can by tagged with roles by appending `=role1,role2,etc.`
    /// An untagged entry is interpreted as the default model.
    ///
    /// Examples:
    ///   -m openrouter/openrouter/free
    ///   -m openrouter/openrouter/free=default,creative
    #[arg(short, long, visible_alias("model"), verbatim_doc_comment)]
    models: Vec<String>,

    /// Default language model temperature
    #[arg(short = 'T', long)]
    temperature: Option<f64>,

    #[command(subcommand)]
    pub command: Command,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    if let Some(env_handle) = &args.env {
        let _ = if env_handle.to_str() == Some("-") {
            dotenvy::from_read(std::io::stdin())
        } else {
            dotenvy::from_path(env_handle)
        };
    }

    match &args.command {
        Command::Check { pretty } => check_workflows(&args, *pretty)?,
        Command::Exec(exec_args) => execute_workflow(&args, exec_args)?,

        #[cfg(feature = "runner-http")]
        Command::Serve(server_args) => service::start_server(&args, server_args)?,

        #[cfg(feature = "runner-mcp")]
        Command::MCP(mcp_args) => mcp_server::start(&args, mcp_args)?,
    }

    Ok(())
}

/// Customization callbacks for the run_loop used by
/// different subcommands to change output handling etc.
#[derive(TypedBuilder)]
struct ExecOverrides {
    #[builder(default=Arc::new(identity))]
    pub run_ctx_fn: Arc<dyn Fn(RunContext) -> RunContext + Send + Sync>,

    #[builder(default=Arc::new(|| {}))]
    pub post_exec_fn: Arc<dyn Fn() + Send + Sync>,

    #[builder(default)]
    pub run_events: RunEventCast,
}

impl Default for ExecOverrides {
    fn default() -> Self {
        Self::builder().build()
    }
}

#[derive(Debug, Default, Clone)]
struct UserData {
    name: String,
    env: HashMap<String, String>,
}

impl UserData {
    #[allow(unused)]
    pub fn with_env(self, env: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            env: env.into_iter().collect(),
            ..self
        }
    }
}

impl From<&str> for UserData {
    fn from(value: &str) -> Self {
        Self {
            name: value.to_string(),
            env: Default::default(),
        }
    }
}

#[derive(TypedBuilder)]
struct ScopeFactory {
    pub models: Arc<BTreeMap<ModelRole, String>>,
    pub workflow_store: Option<WorkflowStoreDir>,
    pub session: ChatSession,
    pub settings: Arc<Preferences>,
    pub rt: tokio::runtime::Handle,
    pub agent_factory: AgentFactory,
    pub shutdown: CancellationToken,

    #[builder(default=Mutex::new(LruCache::new(NonZeroUsize::new(64).unwrap())))]
    pub scopes: Mutex<LruCache<String, Arc<ExecScope>>>,
}

impl ScopeFactory {
    pub fn no_tools(&self) -> Arc<ExecScope> {
        self.scopes
            .lock()
            .unwrap()
            .get_or_insert("###no_tools###".into(), || {
                let agent_factory = self
                    .agent_factory
                    .clone()
                    .with_env(std::env::vars())
                    .with_tools(None);

                Arc::new(
                    ExecScope::builder()
                        .models(self.models.clone())
                        .workflow_store(self.workflow_store.clone())
                        .session(self.session.clone())
                        .settings(self.settings.clone())
                        .rt(self.rt.clone())
                        .agent_factory(agent_factory)
                        .shutdown(self.shutdown.clone())
                        .build(),
                )
            })
            .clone()
    }

    pub fn user_scope(&self, user_data: &UserData) -> Arc<ExecScope> {
        let UserData { name, env } = user_data;
        self.scopes
            .lock()
            .unwrap()
            .get_or_insert(name.clone(), || {
                let env = std::env::vars().chain(env.clone());
                let agent_factory = self.agent_factory.clone().with_env(env);

                let load_results = self.rt.block_on(async { agent_factory.load_tools().await });
                for result in load_results {
                    if let Err(err) = result {
                        tracing::error!(error = %err);
                    }
                }

                Arc::new(
                    ExecScope::builder()
                        .models(self.models.clone())
                        .workflow_store(self.workflow_store.clone())
                        .session(self.session.clone())
                        .settings(self.settings.clone())
                        .rt(self.rt.clone())
                        .agent_factory(agent_factory)
                        .shutdown(self.shutdown.clone())
                        .build(),
                )
            })
            .clone()
    }
}

/// App state common to all subcommands
#[derive(TypedBuilder, Clone)]
struct ExecScope {
    pub models: Arc<BTreeMap<ModelRole, String>>,
    pub workflow_store: Option<WorkflowStoreDir>,
    pub session: ChatSession,
    pub settings: Arc<Preferences>,
    pub rt: tokio::runtime::Handle,
    pub agent_factory: AgentFactory,
    pub shutdown: CancellationToken,
}

/// Instantiate and initialize common objects from global arguments
fn nameless_scope(args: &Args, rt: &tokio::runtime::Handle) -> anyhow::Result<Arc<ExecScope>> {
    Ok(make_scope_factory(args, rt)?.user_scope(&UserData::default()))
}

fn make_scope_factory(args: &Args, rt: &tokio::runtime::Handle) -> anyhow::Result<ScopeFactory> {
    let workflow_store = args
        .workflows
        .as_ref()
        .map(|p| WorkflowStoreDir::init(p, false))
        .transpose()?
        .context("Workflow directory required for server mode")?;
    let tool_store = args.tools.as_ref().map(|p| {
        let store = ToolStore::new(p);
        store.preload_all();
        store
    });
    let session_dir = args
        .session
        .as_ref()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();
    let session = ChatSession::from_dir_name(
        session_dir,
        args.session
            .as_ref()
            .and_then(|s| s.file_stem().map(|s| s.display().to_string()))
            .as_deref(),
    )
    .build()?;
    if let Some(branch) = &args.branch {
        session.transform(|history| Ok(history.switch(branch)))?;
    }
    let settings_path = if let Some(path) = &args.config {
        if !path.is_file() {
            anyhow::bail!("Configuration file does not exist: {path:?}");
        }
        path.clone()
    } else {
        Default::default()
    };
    let mut settings = if settings_path.is_file() {
        let text = std::fs::read_to_string(&settings_path)?;
        toml::from_str(&text)?
    } else {
        Preferences::default()
    };
    tracing::debug!("Loaded settings {settings:?}");
    if let Some(profile) = &args.profile {
        if !settings.has_profile(profile) {
            anyhow::bail!(
                "Profile '{profile}' not found. Valid profiles: {:?}",
                settings.models.keys().collect_vec()
            );
        }

        settings.profile = profile.to_owned();
    }
    if let Some(temperature) = &args.temperature {
        settings.temperature = *temperature;
    }
    let models: BTreeMap<ModelRole, String> = if args.models.is_empty() {
        settings.get_model_map()
    } else {
        tracing::debug!("Setting models from {:?}", &args.models);

        let mut model_map: BTreeMap<ModelRole, String> = Default::default();

        for entry in &args.models {
            tracing::debug!("Parsing model {entry}");

            if let Some((name, roles)) = entry.split_once("=") {
                for role in roles
                    .split(",")
                    .map(|r| ModelRole::from_str(r.trim()).unwrap_or_default())
                {
                    model_map.insert(role.to_owned(), name.to_string());
                }
            } else {
                model_map.insert(ModelRole::Default, entry.trim().to_string());
            }
        }

        model_map
    };
    let models = Arc::new(models);
    let next_workflow: Arc<ArcSwapOption<String>> = Default::default();
    let next_prompt: Arc<ArcSwapOption<String>> = Default::default();
    let next_images: Arc<ArcSwap<Vec<String>>> = Default::default();
    let agent_factory = AgentFactory::builder()
        .rt(rt.clone())
        .prefs(Arc::new(ArcSwap::from_pointee(settings.clone())))
        .tools(tool_store)
        .store(Some(workflow_store.clone()))
        .next_workflow(next_workflow.clone())
        .next_prompt(next_prompt.clone())
        .next_images(next_images.clone())
        .build();

    // // Now loads on new scope
    // let load_results = rt.block_on(async { agent_factory.load_tools().await });
    // for result in load_results {
    //     if let Err(err) = result {
    //         tracing::error!(error = %err);
    //     }
    // }

    Ok(ScopeFactory::builder()
        .models(models)
        .workflow_store(Some(workflow_store))
        .session(session)
        .settings(Arc::new(settings))
        .rt(rt.clone())
        .agent_factory(agent_factory)
        .shutdown(CancellationToken::new())
        .build())
}

/// Runs recursive checks on workflows (i.e. missing tools, etc)
fn check_workflows(args: &Args, pretty: bool) -> anyhow::Result<()> {
    use struson::writer::*;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let exec_scope = nameless_scope(args, rt.handle())?;
    let ExecScope {
        workflow_store,
        agent_factory,
        ..
    } = exec_scope.as_ref();
    let toolbox = &agent_factory.toolbox;

    let Some(store) = workflow_store else {
        anyhow::bail!("Nothing to check");
    };

    let names = store.names().map(|n| n.to_string()).collect_vec();

    let mut json_writer = JsonStreamWriter::new_custom(
        std::io::stdout(),
        WriterSettings {
            pretty_print: pretty,
            ..Default::default()
        },
    );

    json_writer.begin_object()?;

    for name in names {
        let workflow = store.load(&name)?;

        let ctx = CheckContext::builder()
            .toolbox(toolbox.clone())
            .graph_id(workflow.graph.uuid)
            .build();

        let alerts = workflow.graph.check(&ctx);

        if !alerts.is_empty() {
            json_writer.name(&name)?;
            json_writer.begin_array()?;

            for (_, msg) in alerts {
                json_writer.string_value(&msg)?;
            }

            json_writer.end_array()?;
        }
    }

    json_writer.end_object()?;
    json_writer.finish_document()?;

    Ok(())
}

/// Entry point for the `exec` subcommand
fn execute_workflow(args: &Args, exec_args: &ExecArgs) -> anyhow::Result<()> {
    let ExecArgs {
        workflow,
        input,
        input_file,
        images: image,
        out_dir,
        autoruns,
        pretty,
        ..
    } = &exec_args;

    if *autoruns > 0 && args.workflows.is_none() {
        anyhow::bail!("Cannot use autorun without a workflow store");
    }

    if let Some(out_dir) = &out_dir {
        std::fs::create_dir_all(out_dir)?;
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let scaffolding = nameless_scope(args, rt.handle())?;

    let mut prompt = input.as_ref().cloned().unwrap_or_default();
    if &prompt == "-" {
        prompt = std::io::read_to_string(std::io::stdin())?;
    }

    if let Some(path) = &input_file {
        prompt = std::fs::read_to_string(path)?;
    }

    let images = image.clone();
    let workflow_path = Path::new(workflow);

    let graph: Workflow = if workflow == "__default__" {
        Default::default()
    } else if workflow_path.is_file() {
        let reader = OpenOptions::new().read(true).open(workflow_path)?;
        serde_yml::from_reader(reader)?
    } else if let Some(store) = &scaffolding.workflow_store {
        store.load(workflow)?
    } else {
        anyhow::bail!("Invalid file: {workflow:?}");
    };

    let run_count = Arc::new(AtomicUsize::default());

    let saver_task: Arc<Mutex<Option<JoinHandle<anyhow::Result<()>>>>> = Default::default();

    let handle = rt.handle().clone();
    let overrides = ExecOverrides::builder()
        .run_ctx_fn({
            let rt = handle.clone();
            let run_count = run_count.clone();
            let saver_task = saver_task.clone();
            let out_dir = out_dir.clone();
            let autoruns = *autoruns;
            let pretty = *pretty;

            Arc::new(move |run_ctx| {
                let receiver = run_ctx.outputs.receiver();
                let task = if let Some(out_dir) = &out_dir {
                    let out_dir = if autoruns > 0 {
                        out_dir.join(run_count.load(Ordering::Relaxed).to_string())
                    } else {
                        out_dir.clone()
                    };

                    rt.spawn(file_output(receiver, out_dir))
                } else {
                    rt.spawn(console_output(receiver, pretty))
                };

                *saver_task.lock().unwrap() = Some(task);

                run_count.fetch_add(1, Ordering::Relaxed);
                run_ctx
            })
        })
        .post_exec_fn({
            let rt = handle.clone();
            let saver_task = saver_task.clone();

            Arc::new(move || {
                let mut saver_task = saver_task.lock().unwrap();

                if let Some(saver_task) = saver_task.take() {
                    rt.block_on(async move {
                        match saver_task.await {
                            Ok(Ok(_)) => {}
                            Err(err) => {
                                tracing::warn!("{err:?}");
                            }
                            Ok(Err(err)) => {
                                tracing::warn!("{err:?}");
                            }
                        }
                    });
                }
            })
        })
        .build();

    run_loop(&scaffolding, exec_args, overrides, prompt, images, graph)?;

    if args.update_session && args.session.is_some() {
        scaffolding.session.save()?;
    }

    let agent_factory = &scaffolding.agent_factory;
    let next_workflow = agent_factory.next_workflow.clone();
    let next_prompt = agent_factory.next_prompt.clone();
    let next_images = agent_factory.next_images.clone();

    if exec_args.show_next {
        let next_workflow = next_workflow
            .swap(Default::default())
            .map(|s| s.as_ref().clone());
        let next_prompt = next_prompt
            .swap(Default::default())
            .map(|s| s.as_ref().clone());
        let next_images = next_images.swap(Default::default()).as_ref().clone();
        let blob = json!({
            "next_workflow": next_workflow,
            "next_prompt": next_prompt,
            "next_images": next_images,
        });

        println!("{blob}");
    }

    Ok(())
}

/// Template method to run workflows, possibly auto-running chained workflows
fn run_loop(
    scaffolding: &ExecScope,
    exec_args: &ExecArgs,
    overrides: ExecOverrides,
    mut prompt: String,
    mut images: Vec<String>,
    mut workflow: Workflow,
) -> Result<(), anyhow::Error> {
    let autoruns = exec_args.autoruns;
    let event_tx = &overrides.run_events;

    let exec_id = ExecId::random();
    event_tx.broadcast(json!({
        "tags": ["runner"],
        "msg": "starting run loop",
        "exec-id": exec_id,
    }));

    for run_count in 0..=autoruns {
        let ExecScope {
            models,
            workflow_store,
            session,
            settings,
            rt,
            agent_factory,
            shutdown,
            ..
        } = scaffolding;

        let exec_id = exec_id.scope(workflow.id(), run_count);
        let workflow_name = workflow_store
            .as_ref()
            .and_then(|store| {
                use aerie::workflow::store::WorkflowStore as _;
                store.name_for(workflow.id())
            })
            .map(|n| n.to_string());

        event_tx.broadcast(json!({
            "tags": ["workflow"],
            "msg": "initializing workflow",
            "exec-id": exec_id,
            "flow-id": workflow.id(),
            "name": &workflow_name,
        }));

        let mut extra_content = Vec::new();
        for image in &images {
            let image = aerie::rig::message::Image {
                data: aerie::rig::message::DocumentSourceKind::Url(image.into()),
                ..Default::default()
            };
            let image = {
                ImageResolver::builder()
                    .allow_local(run_count == 0)
                    .build()
                    .preprocess(&image)
            }?;
            let content = UserContent::Image(image.into_owned());

            extra_content.push(content);
        }

        let run_ctx = RunContext::builder()
            .runtime(rt.clone())
            .exec_id(exec_id)
            .agent_factory(agent_factory.clone())
            .metadata(workflow.metadata.clone())
            .history(session.history.clone())
            .seed(settings.seed.clone())
            .models(models.clone())
            .run_events(event_tx.clone())
            .build();

        rt.spawn({
            let interrupt = run_ctx.interrupt.clone();
            let shutdown = shutdown.clone();
            async move {
                let _ = shutdown.cancelled().await;
                interrupt.store(true, Ordering::Relaxed);
            }
        });

        let run_ctx = (overrides.run_ctx_fn)(run_ctx);
        let out_rx = run_ctx.outputs.sender();
        let inputs = RootContext::builder()
            .history(session.history.clone())
            .workflow(workflow.clone())
            .user_prompt(std::mem::take(&mut prompt))
            .extra_content(extra_content)
            .model("default".into())
            .temperature(settings.temperature)
            .build()
            .inputs()?;

        let mut exec = WorkflowRunner::builder()
            .inputs(inputs)
            .run_ctx(run_ctx)
            .run_events(overrides.run_events.clone())
            .build();

        exec.init(&workflow.graph);
        let mut snarl = Snarl::try_from(workflow.graph.as_ref().clone())?;

        event_tx.broadcast(json!({
            "tags": ["runner", "workflow"],
            "msg": "starting workflow",
            "exec-id": exec_id,
        }));

        let result = loop {
            match exec.step(&mut snarl) {
                Ok(false) => {
                    exec.root_finish()?;
                    break Ok(false);
                }
                err @ Err(_) => break err,
                _ => {}
            }
        };

        if exec_args.show_ids {
            out_rx
                .send((
                    "__id".into(),
                    Value::Text(Arc::new(workflow.graph.uuid.0.to_string())),
                ))
                .expect("Problem sending graph id");
            out_rx
                .send(("__run".into(), Value::Integer(1 + run_count as i64)))
                .expect("Problem sending graph id");
        }

        drop(out_rx);
        drop(exec);

        (overrides.post_exec_fn)();

        result?;

        event_tx.broadcast(json!({
            "tags": ["workflow"],
            "msg": "finished workflow",
            "exec-id": exec_id,
        }));

        if run_count < autoruns {
            if let Some(next_prompt) = agent_factory.next_prompt.swap(Default::default()) {
                prompt = next_prompt.as_ref().to_owned();
            }

            // TODO: efficiency
            images = agent_factory
                .next_images
                .swap(Default::default())
                .as_ref()
                .clone();

            if let Some(next_workflow) = agent_factory
                .next_workflow
                .swap(Default::default())
                .as_ref()
                && let Some(store) = workflow_store
            {
                workflow = store.load(next_workflow)?;
                event_tx.broadcast(json!({
                    "tags": ["runner", "workflow"],
                    "msg": "chaining workflow",
                    "from": &workflow_name,
                    "next": next_workflow,
                    "run_count": run_count,
                    "autoruns": autoruns,
                }));
            } else {
                break;
            }
        }
    }

    event_tx.broadcast(json!({
        "tags": ["runner"],
        "msg": "finished run loop",
        "exec-id": exec_id,
    }));

    Ok(())
}

#[cfg(feature = "runner-http")]
mod service {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::*;
    use aerie::storage::CachedDirStore;
    use aerie::workflow::OutputChannel;
    use aerie::workflow::store::WorkflowStore;
    use async_stream::stream;
    use axum::extract::{Json, State};
    use axum::http::HeaderMap;
    use axum::response::Sse;
    use axum::response::sse::Event;
    use axum::{
        Router,
        http::StatusCode,
        response::{self, IntoResponse},
        routing::{get, post},
    };
    use glob::Pattern;
    use tokio::task::block_in_place;

    struct MaybeUserData(anyhow::Result<Option<UserData>>);

    impl From<axum::http::HeaderMap> for MaybeUserData {
        fn from(headers: axum::http::HeaderMap) -> Self {
            // TODO: config or build feature?
            if let Some(user_header) = headers.get("X-User-Name") {
                let env = headers
                    .get_all("X-User-Env")
                    .iter()
                    .filter_map(|kv| kv.to_str().ok())
                    .filter_map(|kv| kv.split_once("="))
                    .map(|(k, v)| (k.to_string(), v.to_string()));

                let result = user_header
                    .to_str()
                    .context("Invalid header value")
                    .map(|name| name.into())
                    .map(|ud: UserData| ud.with_env(env))
                    .map(Some);

                return Self(result);
            }

            Self(Ok(None))
        }
    }

    #[derive(Debug, Deserialize)]
    struct ExecutionParams {
        #[serde(default, rename = "#events")]
        events: Vec<String>,

        #[serde(flatten)]
        args: ExecArgs,
    }

    async fn execute_handler(
        headers: HeaderMap,
        State(scope_factory): State<Arc<ScopeFactory>>,
        Json(payload): Json<ExecArgs>,
    ) -> impl IntoResponse {
        let Ok(user_data) = MaybeUserData::from(headers).0 else {
            return StatusCode::BAD_REQUEST.into_response();
        };

        let scaffolding =
            block_in_place(|| scope_factory.user_scope(&user_data.unwrap_or_default()));
        // let scaffolding_ = scaffolding.lock().await;
        let Some(workflow_store) = &scaffolding.workflow_store else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };

        tracing::info!("Payload is {payload:?}");

        let Ok(workflow) = CachedDirStore::load(workflow_store, &payload.workflow) else {
            return StatusCode::NOT_FOUND.into_response();
        };

        let prompt = payload.input.clone().unwrap_or_default();
        let images = payload.images.clone();
        // drop(scaffolding_);

        let (chan_tx, chan_rx) = flume::unbounded();
        let overrides = ExecOverrides::builder()
            .run_ctx_fn({
                Arc::new(move |mut run_ctx| {
                    let outputs = OutputChannel::default();
                    chan_tx
                        .send(outputs.clone())
                        .expect("Could not send output channel");
                    run_ctx.outputs = outputs.clone();
                    run_ctx
                })
            })
            .build();

        let scaffolding = scaffolding.clone();
        let shutdown = scaffolding.shutdown.clone();
        let mut task = tokio::task::spawn_blocking(move || {
            run_loop(&scaffolding, &payload, overrides, prompt, images, workflow)
        });

        let result = tokio::select! {
            res = &mut task => {
                res
            }
            _ = shutdown.cancelled() => {
                tracing::info!("Received shutdown signal. Cancelling run...");
                return (StatusCode::SERVICE_UNAVAILABLE, "Server shutting down").into_response();
            }
        };

        if let Err(err) = result {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("{err:?}")).into_response();
        }

        if let Ok(Err(err)) = result {
            // TODO: more nuance
            return (StatusCode::BAD_REQUEST, format!("{err:?}")).into_response();
        }

        let outputs = chan_rx
            .drain()
            .map(|outputs| {
                outputs
                    .receiver()
                    .drain()
                    .map(|(k, v)| (k, serde_json::Value::try_from(v).unwrap()))
                    .collect::<BTreeMap<_, _>>()
            })
            .collect_vec();

        response::Json(outputs).into_response()
    }

    async fn sse_handler(
        headers: HeaderMap,
        State(scope_factory): State<Arc<ScopeFactory>>,
        Json(payload): Json<ExecutionParams>,
    ) -> impl IntoResponse //Sse<impl Stream<Item = Result<Event, Infallible>>>
    {
        let Ok(user_data) = MaybeUserData::from(headers).0 else {
            return StatusCode::BAD_REQUEST.into_response();
        };

        let scaffolding =
            block_in_place(|| scope_factory.user_scope(&user_data.unwrap_or_default()));
        let Some(workflow_store) = &scaffolding.workflow_store else {
            tracing::error!("Server started without workflow directory");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };

        tracing::info!("Payload is {payload:?}");

        let Ok(workflow) = CachedDirStore::load(workflow_store, &payload.args.workflow) else {
            return StatusCode::NOT_FOUND.into_response();
        };

        let prompt = payload.args.input.clone().unwrap_or_default();
        let images = payload.args.images.clone();

        let (run_tx, mut run_rx) = async_broadcast::broadcast(64);
        run_rx.set_overflow(true);

        let (chan_tx, chan_rx) = flume::unbounded();
        let overrides = ExecOverrides::builder()
            .run_ctx_fn({
                Arc::new(move |mut run_ctx| {
                    let outputs = OutputChannel::default();
                    chan_tx
                        .send(outputs.clone())
                        .expect("Could not send output channel");
                    run_ctx.outputs = outputs.clone();
                    run_ctx
                })
            })
            .run_events(RunEventCast(run_tx))
            .build();

        let start = Instant::now();
        let scaffolding = scaffolding.clone();
        let mut task = tokio::task::spawn_blocking(move || {
            run_loop(
                &scaffolding,
                &payload.args,
                overrides,
                prompt,
                images,
                workflow,
            )
        });

        let event_filters = payload
            .events
            .iter()
            .filter_map(|s| Pattern::new(s).ok())
            .collect_vec();

        let resp_stream = stream! {
            let mut outputs = OutputChannel::default();
            let mut out_rx = outputs.receiver();

            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {
                        let elapsed = start.elapsed().as_secs();
                        yield Ok(Event::default().comment(format!("keep-alive @ {elapsed}s")));
                    }
                    Ok(out_chan) = chan_rx.recv_async() => {
                        outputs = out_chan;
                        out_rx = outputs.receiver();
                        yield Ok(Event::default().comment("Switching to new output channel"));
                    }
                    Ok((k,v)) = out_rx .recv_async() => {
                        yield Event::default().event("output").json_data(json!({k: v}));
                    }
                    Ok(event) = run_rx.recv() => {
                         if event.filter(&event_filters) {
                             yield Event::default().event("run-event").json_data(
                                event.with_elapsed(start.elapsed()));
                         }
                    }
                    res = &mut task => {
                        let elapsed = start.elapsed().as_secs_f32();
                        let msg = format!("Finished task in {elapsed:.2}s with result: {res:?}");
                        tracing::info!("{msg}");
                        yield Ok::<_, axum::Error>(Event::default().comment(msg.escape_default().to_string()));
                        break;
                    }
                }
            }
        };

        Sse::new(resp_stream).into_response()
    }

    pub fn start_server(args: &Args, server_args: &HttpServerArgs) -> anyhow::Result<()> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        let scope_factory = Arc::new(make_scope_factory(args, rt.handle())?);

        // TODO: require workflow store

        let shutdown = scope_factory.shutdown.clone();
        rt.spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("Received interrupt. Beginning shutdown sequence...");
            shutdown.cancel();
        });

        let shutdown = scope_factory.shutdown.clone();

        rt.block_on(async move {
            let app = Router::new()
                .route(
                    "/list",
                    get({
                        let scope_factory = scope_factory.clone();
                        || async move {
                            let scope = scope_factory.no_tools();
                            // let scaffolding_ = scaffolding.lock().await;
                            let Some(workflow_store) = &scope.workflow_store else {
                                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                            };

                            response::Json(
                                WorkflowStore::names(workflow_store)
                                    .map(|s| s.to_string())
                                    .collect_vec(),
                            )
                            .into_response()
                        }
                    }),
                )
                .route("/execute", post(execute_handler))
                .route("/sse", post(sse_handler))
                .with_state(scope_factory.clone());

            let HttpServerArgs { host, port } = server_args;
            let addr = format!("{host}:{port}");
            let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
            println!("Listening on {addr}");

            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown.cancelled().await;
                })
                .await
                .unwrap();

            tracing::info!("Server stopped.");

            Ok(())
        })
    }
}

#[cfg(feature = "runner-mcp")]
mod mcp_server {
    use super::{ExecOverrides, ExecScope, run_loop};

    use aerie::{
        toolbox::parse_or_prompt_schema,
        workflow::{OutputChannel, store::WorkflowStore},
    };
    use itertools::Itertools;
    use rmcp::{
        RoleServer, ServerHandler,
        model::{ErrorData as McpError, PaginatedRequestParams, *},
        service::RequestContext,
    };
    use std::{collections::BTreeMap, option::Option, sync::Arc};
    use typed_builder::TypedBuilder;

    #[derive(TypedBuilder)]
    pub struct RunnerService {
        scaffolding: Arc<ExecScope>,

        #[builder(default)]
        autoruns: usize,
    }

    #[allow(clippy::manual_async_fn)] // Can't get lifetimes to agree when using async-trait
    impl ServerHandler for RunnerService {
        fn get_info(&self) -> ServerInfo {
            ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
                .with_protocol_version(ProtocolVersion::V_2024_11_05)
                .with_server_info(Implementation::from_build_env())
                .with_instructions("A service to run agentic workflows.")
        }

        fn get_tool(&self, name: &str) -> Option<Tool> {
            let scaffolding = self.scaffolding.clone();
            scaffolding.workflow_store.as_ref().map(|store| {
                let desc = store.description(name);
                let schema = store.schema(name);
                let schema = object(parse_or_prompt_schema(&schema));

                Tool::new(name.to_string(), desc.into_owned(), Arc::new(schema))
            })
        }

        fn list_tools(
            &self,
            _request: Option<PaginatedRequestParams>,
            _context: RequestContext<RoleServer>,
        ) -> impl Future<Output = Result<ListToolsResult, McpError>> {
            async move {
                let scaffolding = self.scaffolding.clone();
                let tools = if let Some(store) = &scaffolding.workflow_store {
                    store
                        .names()
                        .map(|name| {
                            let desc = store.description(&name);
                            let schema = store.schema(&name);
                            let schema = object(parse_or_prompt_schema(&schema));

                            Tool::new(name.into_owned(), desc.into_owned(), Arc::new(schema))
                        })
                        .collect_vec()
                } else {
                    vec![]
                };

                Ok(ListToolsResult {
                    tools,
                    ..Default::default()
                })
            }
        }

        fn call_tool(
            &self,
            request: CallToolRequestParams,
            _context: RequestContext<RoleServer>,
        ) -> impl Future<Output = Result<CallToolResult, McpError>> {
            Box::pin(async move {
                let scaffolding = self.scaffolding.clone();
                let store = scaffolding
                    .workflow_store
                    .as_ref()
                    .ok_or(McpError::internal_error("Workflow store missing", None))?;

                let workflow = store.load(&request.name).map_err(|_| {
                    McpError::new(ErrorCode::METHOD_NOT_FOUND, request.name.clone(), None)
                })?;

                let schema = store.schema(&request.name);

                let (prompt, images) = if !schema.is_empty()
                    && let Ok(schema) = serde_json::from_str(&schema)
                {
                    // Should we bail or just ignore invalid schemas?
                    let validator = jsonschema::validator_for(&schema).map_err(|err| {
                        McpError::internal_error(format!("Invalid schema: {err:?}"), None)
                    })?;

                    let args = if let Some(args) = request.arguments.as_ref() {
                        args
                    } else {
                        &Default::default()
                    };

                    let input =
                        serde_json::to_value(args).expect("Could not convert input map to object");

                    validator.validate(&input).map_err(|err| {
                        McpError::invalid_request(format!("Validation error: {err:?}"), None)
                    })?;

                    (input, Default::default())
                } else {
                    let prompt = request
                        .arguments
                        .as_ref()
                        .and_then(|args| args.get("prompt").cloned())
                        .unwrap_or_default();
                    let images = request
                        .arguments
                        .as_ref()
                        .and_then(|args| args.get("images").cloned())
                        .and_then(|imgs| serde_json::from_value(imgs).ok())
                        .unwrap_or_default();
                    (prompt, images)
                };

                let prompt = serde_json::to_string(&prompt)
                    .map_err(|e| McpError::internal_error(format!("{e:?}"), None))?;

                let (chan_tx, chan_rx) = flume::unbounded();
                let overrides = ExecOverrides::builder()
                    .run_ctx_fn({
                        Arc::new(move |mut run_ctx| {
                            let outputs = OutputChannel::default();
                            chan_tx
                                .send(outputs.clone())
                                .expect("Could not send output channel");
                            run_ctx.outputs = outputs.clone();
                            run_ctx
                        })
                    })
                    .build();

                // TODO: per request autoruns
                let exec_args = super::ExecArgs {
                    autoruns: self.autoruns,
                    ..Default::default()
                };

                let exec_task = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                    run_loop(
                        &scaffolding,
                        &exec_args,
                        overrides,
                        prompt,
                        images,
                        workflow,
                    )?;

                    let outputs = chan_rx
                        .drain()
                        .map(|outputs| {
                            outputs
                                .receiver()
                                .drain()
                                .map(|(k, v)| (k, serde_json::Value::try_from(v).unwrap()))
                                .collect::<BTreeMap<_, _>>()
                        })
                        .collect_vec();

                    drop(exec_args);
                    drop(scaffolding);
                    Ok(outputs)
                });

                let result = exec_task.await;

                let outputs: Result<Vec<_>, serde_json::Error> = match result {
                    Err(err) => Err(McpError::internal_error(
                        format!("Unable to launch runner: {err:?}"),
                        None,
                    ))?,
                    Ok(Err(err)) => Err(McpError::internal_error(
                        format!("Error during workflow execution: {err:?}"),
                        None,
                    ))?,
                    Ok(Ok(outputs)) => outputs
                        .into_iter()
                        .map(|obj| serde_json::to_value(&obj))
                        .collect(),
                };

                let content = outputs
                    .map_err(|err| McpError::internal_error(format!("{err:?}"), None))?
                    .into_iter()
                    .map(Content::json)
                    .collect::<Result<Vec<_>, McpError>>()?;

                Ok(CallToolResult::success(content))
            })
        }
    }

    pub fn start(args: &crate::Args, mcp_args: &crate::McpServerArgs) -> anyhow::Result<()> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        let scaffolding = super::nameless_scope(args, rt.handle())?;

        let shutdown = scaffolding.shutdown.clone();
        rt.spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("Received interrupt. Beginning shutdown sequence...");
            shutdown.cancel();
            tracing::info!("Shutdown flag set 2");
            // tracing::info!("Stopping server");
            // token.cancel();
        });

        let result = rt.block_on(async move {
            use rmcp::{ServiceExt, transport::stdio};
            let shutdown = scaffolding.shutdown.clone();
            let runner_service = RunnerService::builder()
                .scaffolding(scaffolding.clone())
                .autoruns(mcp_args.autoruns)
                .build();

            let service = runner_service.serve_with_ct(stdio(), shutdown).await?;
            service.waiting().await?;

            Ok(())
        });

        tracing::info!("Server stopped");
        result
    }
}

/// Worker task to dump outputs to console as a streamed JSON object
async fn console_output(
    out_rx: flume::Receiver<(String, Value)>,
    pretty: bool,
) -> anyhow::Result<()> {
    use aerie::workflow::Value;
    use struson::writer::*;

    let mut json_writer = JsonStreamWriter::new_custom(
        std::io::stdout(),
        WriterSettings {
            pretty_print: pretty,
            ..Default::default()
        },
    );

    json_writer.begin_object()?;
    while let Ok((label, value)) = out_rx.recv_async().await {
        json_writer.name(&label)?;

        match value {
            Value::Text(text) => {
                json_writer.string_value(&text)?;
            }
            Value::Number(value) => json_writer.fp_number_value(value.into_inner())?,
            Value::Integer(value) => json_writer.number_value(value)?,
            Value::Json(value) => json_writer.serialize_value(&value)?,
            Value::Chat(value) => json_writer.serialize_value(&value)?,
            Value::Message(message) => json_writer.string_value(&message_text(&message))?,
            Value::TextList(value) => json_writer.serialize_value(&value)?,
            Value::FloatList(value) => json_writer.serialize_value(&value)?,
            Value::IntList(value) => json_writer.serialize_value(&value)?,
            Value::MsgList(value) => json_writer.serialize_value(&value)?,
            _ => {
                json_writer.serialize_value(&value)?;
            }
        }
    }

    json_writer.end_object()?;
    json_writer.finish_document()?;
    println!();

    Ok(())
}

/// Worker task to dump workflow outputs to distinct files in a directory
async fn file_output(
    out_rx: flume::Receiver<(String, Value)>,
    path: PathBuf,
) -> anyhow::Result<()> {
    while let Ok((label, value)) = out_rx.recv_async().await {
        let path = path.join(label);

        let fh = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        // if out_glob.matches(&label) {
        write_value(fh, &value)?;
        // }
    }

    Ok(())
}
