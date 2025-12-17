use std::{
    fs::OpenOptions,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use aerie::{
    AgentFactory, ChatSession, Settings,
    utils::message_text,
    workflow::{RunContext, ShadowGraph, WorkNode, runner::WorkflowRunner, write_value},
};
use clap::Parser;
use egui_snarl::Snarl;
use itertools::Itertools as _;
use serde::Serializer as _;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// A minimalist workflow runner that dumps outputs to the console as a JSON object.
///
/// If you need post-processing, use external tools like jq, sed and awk.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// The workflow file to run
    workflow: PathBuf,

    /// Save outputs as individual files in a directory
    #[arg(short, long)]
    out_dir: Option<PathBuf>,

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
    #[arg(short, long)]
    prompt: Option<String>,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    if let Some(out_dir) = &args.out_dir {
        std::fs::create_dir_all(out_dir)?;
    }

    // let pattern = args.filter.as_ref().cloned().unwrap_or("*".into());
    // let out_glob = Pattern::new(&pattern)?;

    let workflow_path = args.workflow.as_path();
    if !workflow_path.is_file() {
        anyhow::bail!("Invalid file: {workflow_path:?}");
    }

    let reader = OpenOptions::new().read(true).open(workflow_path)?;
    let shadow: ShadowGraph<WorkNode> = serde_yml::from_reader(reader)?;

    let session = ChatSession::load(args.session.as_ref()).build()?;
    if let Some(branch) = &args.branch {
        session.transform(|history| Ok(history.switch(branch)))?;
    }

    let settings_path = if let Some(path) = &args.config {
        if !path.is_file() {
            anyhow::bail!("Configuration file does not exist: {path:?}");
        }
        path.clone()
    } else {
        dirs::config_dir()
            .map(|p| p.join("emberlain"))
            .unwrap_or_default()
            .join("workbench.yml")
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

    let mut agent_factory = AgentFactory::builder()
        .rt(rt.handle().clone())
        .settings(Arc::new(RwLock::new(settings.clone())))
        .build();

    agent_factory.reload_tools()?;

    let run_ctx = RunContext::builder()
        .agent_factory(agent_factory)
        .history(session.history.clone())
        .user_prompt(args.prompt.as_ref().cloned().unwrap_or_default())
        .model(settings.llm_model.clone())
        .build();

    let saver_task = if let Some(out_dir) = &args.out_dir {
        rt.handle()
            .spawn(file_output(run_ctx.outputs.receiver(), out_dir.clone()))
    } else {
        rt.handle()
            .spawn(console_output(run_ctx.outputs.receiver()))
    };

    let mut exec = WorkflowRunner::builder().run_ctx(run_ctx).build();

    exec.init(&shadow);
    let mut snarl = Snarl::try_from(shadow)?;

    let result: anyhow::Result<()> = rt.block_on(async move {
        loop {
            if !exec.step(&mut snarl).await? {
                break Ok(());
            }
        }
    });

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
