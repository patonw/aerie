use std::{
    collections::BTreeMap,
    convert::identity,
    fs::OpenOptions,
    path::{Path, PathBuf},
    str::FromStr as _,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::JoinHandle,
};

use aerie::{
    AgentFactory, ChatSession, Preferences,
    config::ModelRole,
    rig::message::UserContent,
    storage::CachedDirStore as _,
    toolbox::ToolStore,
    utils::{ImageResolver, message_text},
    workflow::{
        CheckContext, RootContext, RunContext, Value, Workflow, runner::WorkflowRunner,
        store::WorkflowStoreDir, write_value,
    },
};
use arc_swap::{ArcSwap, ArcSwapOption};
use clap::{Parser, Subcommand};
use egui_snarl::Snarl;
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_with::skip_serializing_none;
use serde_yaml_ng as serde_yml;
use tokio::runtime::Runtime;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};
use typed_builder::TypedBuilder;

#[skip_serializing_none]
#[derive(clap::Args, Clone, Debug, Serialize, Deserialize)]
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
    #[arg(short = 'n', long, action = clap::ArgAction::SetTrue, default_value_t = false)]
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
}

impl Default for ExecOverrides {
    fn default() -> Self {
        Self::builder().build()
    }
}

/// App state common to all subcommands
struct Scaffolding<'a> {
    pub models: Arc<BTreeMap<ModelRole, String>>,
    pub workflow_store: Option<WorkflowStoreDir>,
    pub session: ChatSession,
    pub settings: Preferences,
    pub rt: Runtime,
    pub ex: smol::LocalExecutor<'a>,
    pub agent_factory: AgentFactory,
}

/// Instantiate and initialize common objects from global arguments
fn make_scaffolding(args: &Args) -> anyhow::Result<Scaffolding<'_>> {
    let workflow_store = args
        .workflows
        .as_ref()
        .map(|p| WorkflowStoreDir::load_all(p, false))
        .transpose()?;
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
        // dirs::config_dir()
        //     .map(|p| p.join("aerie"))
        //     .unwrap_or_default()
        //     .join("workbench.yml")
    };
    let mut settings = if settings_path.is_file() {
        let text = std::fs::read_to_string(&settings_path)?;
        serde_yml::from_str(&text)?
    } else {
        Preferences::default()
    };
    tracing::debug!("Loaded settings {settings:?}");
    if let Some(profile) = &args.profile {
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

    // let ex = smol::LocalExecutor::new();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;

    // The chain tool requires these. It's an impediment to concurrent execution.
    // Need to scope this by run somehow
    let next_workflow: Arc<ArcSwapOption<String>> = Default::default();
    let next_prompt: Arc<ArcSwapOption<String>> = Default::default();
    let next_images: Arc<ArcSwap<Vec<String>>> = Default::default();

    let agent_factory = AgentFactory::builder()
        .rt(rt.handle().clone())
        .prefs(Arc::new(ArcSwap::from_pointee(settings.clone())))
        .tools(tool_store)
        .store(workflow_store.clone())
        .next_workflow(next_workflow.clone())
        .next_prompt(next_prompt.clone())
        .next_images(next_images.clone())
        .build();

    let load_results = smol::block_on(async { agent_factory.load_tools().await });
    for result in load_results {
        if let Err(err) = result {
            tracing::error!(error = %err);
        }
    }

    loop {
        // Probably not necessary now
        let num_tasks = agent_factory.task_count.load(Ordering::Relaxed);
        if num_tasks < 1 {
            break;
        }

        tracing::info!("Waiting for {num_tasks} tools to load...");
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    Ok(Scaffolding {
        models,
        workflow_store,
        session,
        settings,
        rt,
        ex: Default::default(),
        agent_factory,
    })
}

/// Runs recursive checks on workflows (i.e. missing tools, etc)
fn check_workflows(args: &Args, pretty: bool) -> anyhow::Result<()> {
    use struson::writer::*;
    let Scaffolding {
        workflow_store,
        agent_factory,
        ..
    } = make_scaffolding(args)?;
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

    let mut scaffolding = make_scaffolding(args)?;

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
    } else if let Some(store) = &mut scaffolding.workflow_store {
        store.load(workflow)?
    } else {
        anyhow::bail!("Invalid file: {workflow:?}");
    };

    let run_count = Arc::new(AtomicUsize::default());

    let saver_task: Arc<Mutex<Option<JoinHandle<anyhow::Result<()>>>>> = Default::default();

    let overrides = ExecOverrides::builder()
        .run_ctx_fn({
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

                    std::thread::spawn(move || file_output(receiver, out_dir))
                } else {
                    std::thread::spawn(move || console_output(receiver, pretty))
                };

                *saver_task.lock().unwrap() = Some(task);

                run_count.fetch_add(1, Ordering::Relaxed);
                run_ctx
            })
        })
        .post_exec_fn({
            let saver_task = saver_task.clone();

            Arc::new(move || {
                let mut saver_task = saver_task.lock().unwrap();

                if let Some(saver_task) = saver_task.take() {
                    match saver_task.join() {
                        Ok(Ok(_)) => {}
                        Err(err) => {
                            tracing::warn!("{err:?}");
                        }
                        Ok(Err(err)) => {
                            tracing::warn!("{err:?}");
                        }
                    }
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
    scaffolding: &Scaffolding,
    exec_args: &ExecArgs,
    overrides: ExecOverrides,
    mut prompt: String,
    mut images: Vec<String>,
    mut workflow: Workflow,
) -> Result<(), anyhow::Error> {
    let autoruns = exec_args.autoruns;

    for run_count in 0..=autoruns {
        let Scaffolding {
            models,
            workflow_store,
            session,
            settings,
            rt,
            agent_factory,
            ..
        } = scaffolding;

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
            .runtime(rt.handle().clone())
            .exec_id(workflow.graph.uuid.into())
            .agent_factory(agent_factory.clone())
            .metadata(workflow.metadata.clone())
            .history(session.history.clone())
            .seed(settings.seed.clone())
            .models(models.clone())
            .build();

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
            .build();

        exec.init(&workflow.graph);
        let mut snarl = Snarl::try_from(workflow.graph.as_ref().clone())?;

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
            } else {
                break;
            }
        }
    }

    Ok(())
}

/// Worker task to dump outputs to console as a streamed JSON object
fn console_output(out_rx: flume::Receiver<(String, Value)>, pretty: bool) -> anyhow::Result<()> {
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
    while let Ok((label, value)) = out_rx.recv() {
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
fn file_output(out_rx: flume::Receiver<(String, Value)>, path: PathBuf) -> anyhow::Result<()> {
    while let Ok((label, value)) = out_rx.recv() {
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
