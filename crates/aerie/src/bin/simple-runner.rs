use std::{fs::OpenOptions, path::PathBuf, sync::Arc};

use aerie::{
    AgentFactory, ChatSession, Settings,
    utils::message_text,
    workflow::{
        RunContext, ShadowGraph, WorkNode,
        runner::WorkflowRunner,
        store::{WorkflowStore as _, WorkflowStoreDir},
        write_value,
    },
};
use arc_swap::{ArcSwap, ArcSwapOption};
use clap::Parser;
use egui_snarl::Snarl;
use itertools::Itertools as _;
use serde::Serializer as _;
use serde_json::json;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// A minimalist workflow runner that dumps outputs to the console as a JSON object.
///
/// If you need post-processing, use external tools like jq, sed and awk.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// The workflow file to run
    workflow: PathBuf,

    /// Path to a workflow directory for chain execution
    #[arg(short, long)]
    workstore: Option<PathBuf>,

    /// A session to use in the workflow.
    /// Updates are discarded unless `--update` is also used.
    #[arg(short, long)]
    session: Option<PathBuf>,

    /// Configuration file containing tool providers and default agent settings
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// The session branch to use
    #[arg(short, long)]
    branch: Option<String>,

    /// Save updates to the session after running the workflow.
    #[arg(long, action)]
    update: bool,

    /// The default model for the workflow. Has no effect on nodes that define a specific model.
    #[arg(short, long)]
    model: Option<String>,

    #[arg(short, long)]
    temperature: Option<f64>,

    /// Initial user prompt if required by the workflow.
    #[arg(short, long, visible_alias("prompt"))]
    input: Option<String>,

    /// Save outputs as individual files in a directory
    #[arg(short, long)]
    out_dir: Option<PathBuf>,

    /// Number of extra turns to run chained workflows
    #[arg(short, long, default_value_t = 0)]
    autoruns: usize,

    /// Prints an additional object containing the next workflow after the last run
    #[arg(short, long, action = clap::ArgAction::SetTrue, default_value_t = false)]
    next: bool,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    if args.autoruns > 0 && args.workstore.is_none() {
        anyhow::bail!("Cannot use autorun without a workflow store");
    }

    if let Some(out_dir) = &args.out_dir {
        std::fs::create_dir_all(out_dir)?;
    }

    let mut workflow_store = args
        .workstore
        .map(|p| WorkflowStoreDir::load_all(p, false))
        .transpose()?;

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

    // Runtime settings:
    let mut settings = if settings_path.is_file() {
        let text = std::fs::read_to_string(&settings_path)?;
        serde_yml::from_str(&text)?
    } else {
        Settings::default()
    };

    tracing::debug!("Loaded settings {settings:?}");

    if let Some(model) = &args.model {
        settings.llm_model = model.clone();
    }

    if let Some(temperature) = &args.temperature {
        settings.temperature = *temperature;
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;

    let next_workflow: Arc<ArcSwapOption<String>> = Default::default();
    let next_prompt: Arc<ArcSwapOption<String>> = Default::default();
    let mut agent_factory = AgentFactory::builder()
        .rt(rt.handle().clone())
        .settings(Arc::new(ArcSwap::from_pointee(settings.clone())))
        .store(workflow_store.clone())
        .next_workflow(next_workflow.clone())
        .next_prompt(next_prompt.clone())
        .build();

    agent_factory.reload_tools()?;

    let mut prompt = args.input.as_ref().cloned().unwrap_or_default();
    let workflow_path = args.workflow.as_path();
    let mut shadow: ShadowGraph<WorkNode> = if workflow_path.is_file() {
        let reader = OpenOptions::new().read(true).open(workflow_path)?;
        serde_yml::from_reader(reader)?
    } else if let Some(store) = &mut workflow_store {
        store.load(&args.workflow.display().to_string())?
    } else {
        anyhow::bail!("Invalid file: {workflow_path:?}");
    };

    for run_count in 0..=args.autoruns {
        let run_ctx = RunContext::builder()
            .runtime(rt.handle().clone())
            .agent_factory(agent_factory.clone())
            .history(session.history.clone())
            .graph(shadow.clone())
            .user_prompt(prompt.clone())
            .model(settings.llm_model.clone())
            .temperature(settings.temperature)
            .seed(settings.seed.clone())
            .build();

        let saver_task = if let Some(out_dir) = &args.out_dir {
            let out_dir = if args.autoruns > 0 {
                out_dir.join(run_count.to_string())
            } else {
                out_dir.clone()
            };

            rt.handle()
                .spawn(file_output(run_ctx.outputs.receiver(), out_dir))
        } else {
            rt.handle()
                .spawn(console_output(run_ctx.outputs.receiver()))
        };

        let mut exec = WorkflowRunner::builder().run_ctx(run_ctx).build();

        exec.init(&shadow);
        let mut snarl = Snarl::try_from(shadow.clone())?;

        let result = loop {
            match exec.step(&mut snarl) {
                Ok(false) => break Ok(false),
                err @ Err(_) => break err,
                _ => {}
            }
        };
        drop(exec);

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

        result?;

        if args.update && args.session.is_some() {
            session.save()?;
        }

        if run_count < args.autoruns {
            if let Some(next_prompt) = next_prompt.swap(Default::default()) {
                prompt = next_prompt.as_ref().to_owned();
            }

            if let Some(next_workflow) = next_workflow.swap(Default::default()).as_ref()
                && let Some(store) = &mut workflow_store
            {
                shadow = store.load(next_workflow)?;
            } else {
                break;
            }
        }
    }

    if args.next {
        let next_workflow = next_workflow
            .swap(Default::default())
            .map(|s| s.as_ref().clone());
        let next_prompt = next_prompt
            .swap(Default::default())
            .map(|s| s.as_ref().clone());
        let blob = json!({
            "next_workflow": next_workflow,
            "next_prompt": next_prompt,
        });

        println!("{blob}");
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
            aerie::workflow::Value::Chat(chat) => {
                let value = chat.iter_msgs().map(|it| it.into_owned()).collect_vec();
                mapper.serialize_entry(&label, &value).unwrap()
            }
            aerie::workflow::Value::Message(message) => {
                let text = message_text(&message);

                mapper.serialize_entry(&label, &text).unwrap();
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
