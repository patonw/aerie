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
};

use aerie::{
    AgentFactory, ChatSession, Settings,
    config::ModelRole,
    rig::message::UserContent,
    storage::CachedDirStore as _,
    toolbox::ToolStore,
    utils::{ImageResolver, message_text},
    workflow::{
        RootContext, RunContext, Value, Workflow,
        runner::WorkflowRunner,
        store::{WorkflowStore as _, WorkflowStoreDir},
        write_value,
    },
};
use arc_swap::{ArcSwap, ArcSwapOption};
use clap::{Parser, Subcommand};
use egui_snarl::Snarl;
use serde::{Deserialize, Serialize, Serializer as _};
use serde_json::json;
use serde_with::skip_serializing_none;
use serde_yaml_ng as serde_yml;
use tokio::{runtime::Runtime, task::JoinHandle};
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
}

#[inline]
fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[non_exhaustive]
#[derive(Subcommand, Clone, Debug)]
pub enum Command {
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
        Command::Exec(exec_args) => execute_workflow(&args, exec_args)?,
    }

    Ok(())
}

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

struct Scaffolding {
    pub models: Arc<BTreeMap<ModelRole, String>>,
    pub workflow_store: Option<WorkflowStoreDir>,
    pub session: ChatSession,
    pub settings: Settings,
    pub rt: Runtime,
    pub agent_factory: AgentFactory,
}

fn make_scaffolding(args: &Args) -> anyhow::Result<Scaffolding> {
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
        Settings::default()
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

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;

    // The chain tool requires these. It's an impediment to concurrent execution.
    // Need to scope this by run somehow
    let next_workflow: Arc<ArcSwapOption<String>> = Default::default();
    let next_prompt: Arc<ArcSwapOption<String>> = Default::default();
    let next_images: Arc<ArcSwap<Vec<String>>> = Default::default();

    let mut agent_factory = AgentFactory::builder()
        .rt(rt.handle().clone())
        .settings(Arc::new(ArcSwap::from_pointee(settings.clone())))
        .tools(tool_store)
        .store(workflow_store.clone())
        .next_workflow(next_workflow.clone())
        .next_prompt(next_prompt.clone())
        .next_images(next_images.clone())
        .build();
    agent_factory.reload_tools()?;

    // TODO: better synchronization mechanism for waiting on tools to load
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));

        let num_tasks = agent_factory.task_count.load(Ordering::Relaxed);
        if num_tasks < 1 {
            break;
        }

        tracing::info!("Waiting for {num_tasks} tools to load...");
    }

    // let overrides = ExecOverrides {
    //     settings_fn: Arc::new(move |mut settings| {
    //         settings.llm_model = format!("{:?}", next_workflow);
    //         settings
    //     }),
    // };

    Ok(Scaffolding {
        models,
        workflow_store,
        session,
        settings,
        rt,
        agent_factory,
    })
}

fn execute_workflow(args: &Args, exec_args: &ExecArgs) -> anyhow::Result<()> {
    let ExecArgs {
        workflow,
        input,
        input_file,
        images: image,
        out_dir,
        autoruns,
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
            let rt = scaffolding.rt.handle().clone();
            let run_count = run_count.clone();
            let saver_task = saver_task.clone();
            let out_dir = out_dir.clone();
            let autoruns = *autoruns;

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
                    rt.spawn(console_output(receiver))
                };

                *saver_task.lock().unwrap() = Some(task);

                run_count.fetch_add(1, Ordering::Relaxed);
                run_ctx
            })
        })
        .post_exec_fn({
            let rt = scaffolding.rt.handle().clone();
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

async fn console_output(
    out_rx: flume::Receiver<(String, aerie::workflow::Value)>,
) -> anyhow::Result<()> {
    use serde::ser::SerializeMap as _;
    let mut serializer = serde_json::Serializer::pretty(std::io::stdout());
    let mut mapper = serializer.serialize_map(None).unwrap();

    while let Ok((label, value)) = out_rx.recv_async().await {
        // if out_glob.matches(&label) {
        match value {
            aerie::workflow::Value::Text(text) => {
                mapper.serialize_entry(&label, &text).unwrap();
            }
            aerie::workflow::Value::Number(value) => {
                mapper.serialize_entry(&label, &value).unwrap()
            }
            aerie::workflow::Value::Integer(value) => {
                mapper.serialize_entry(&label, &value).unwrap()
            }
            aerie::workflow::Value::Json(value) => mapper.serialize_entry(&label, &value).unwrap(),
            aerie::workflow::Value::Chat(chat) => mapper.serialize_entry(&label, &chat).unwrap(),
            aerie::workflow::Value::Message(message) => {
                let text = message_text(&message);

                mapper.serialize_entry(&label, &text).unwrap();
            }
            aerie::workflow::Value::TextList(value) => {
                mapper.serialize_entry(&label, &value).unwrap()
            }
            aerie::workflow::Value::FloatList(value) => {
                mapper.serialize_entry(&label, &value).unwrap()
            }
            aerie::workflow::Value::IntList(value) => {
                mapper.serialize_entry(&label, &value).unwrap()
            }
            aerie::workflow::Value::MsgList(value) => {
                mapper.serialize_entry(&label, &value).unwrap()
            }
            _ => {
                mapper.serialize_entry(&label, &value).unwrap();
            }
        }
        // }
    }
    mapper.end().unwrap();
    println!();

    Ok(())
}

async fn file_output(
    out_rx: flume::Receiver<(String, aerie::workflow::Value)>,
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
