use std::{
    convert::identity,
    fs::OpenOptions,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use aerie::{
    rig::message::UserContent,
    storage::CachedDirStore as _,
    utils::ImageResolver,
    workflow::{
        CheckContext, RootContext, RunContext, Value, Workflow,
        runner::{ExecId, RunEventCast, WorkflowRunner},
    },
};
use egui_snarl::Snarl;
use itertools::Itertools as _;
use serde_json::json;
use serde_yaml_ng as serde_yml;
use tokio::task::JoinHandle;
use typed_builder::TypedBuilder;

use crate::{
    cli::{Args, ExecArgs},
    output::{console_output, file_output},
    scoping::{ExecScope, nameless_scope},
};

/// Customization callbacks for the run_loop used by
/// different subcommands to change output handling etc.
#[derive(TypedBuilder)]
pub struct ExecOverrides {
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

/// Runs recursive checks on workflows (i.e. missing tools, etc)
pub fn check_workflows(args: &Args, pretty: bool) -> anyhow::Result<()> {
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
pub fn execute_workflow(args: &Args, exec_args: &ExecArgs) -> anyhow::Result<()> {
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
pub fn run_loop(
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
