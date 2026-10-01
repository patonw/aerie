#[cfg(feature = "native")]
use arc_swap::ArcSwap;
#[cfg(feature = "native")]
use itertools::Itertools as _;
#[cfg(feature = "native")]
use scopeguard::defer;
#[cfg(feature = "native")]
use std::{
    sync::{Arc, atomic::Ordering},
    thread,
    time::{Duration, SystemTime},
};

#[cfg(feature = "native")]
use crate::{
    config::ConfigExt as _,
    events::{ExecEvent, WorkflowEvent},
    rig,
    state::app::AppState,
    utils::{ErrorDistiller as _, ImageResolver},
    workflow::{
        RootContext, RunContext,
        runner::{WorkflowRun, WorkflowRunner},
    },
};

#[cfg(feature = "native")]
impl AppState {
    /// Runs the workflow currently being edited and updates nodes in the viewer with results.
    pub fn exec_workflow(&mut self) {
        let tx = self.workflows.broadcast.clone();
        let mut target = self.workflows.view_stack.root_snarl().unwrap();
        let notify = self.notify.clone();
        let task_count_ = self.task_count.clone();
        task_count_.fetch_add(1, Ordering::Relaxed);

        let prompt = self.prompt.clone();

        let _ = self
            .state
            .update(|state| state.workflow = self.workflows.editing.clone());

        self.data.session.scratch.clear();

        let model_map = self.data.prefs.view(|s| s.get_model_map());

        let exec_id = self.workflows.shadow.graph.uuid.into();
        let mut exec = {
            let run_ctx = RunContext::builder()
                .runtime(self.rt.clone())
                .exec_id(exec_id)
                .agent_factory(self.agent_factory.clone())
                .metadata(self.workflows.shadow.metadata.clone())
                .events(Some(self.events.clone()))
                .node_state(self.workflows.node_state.clone())
                .previews(self.workflows.previews.clone())
                .transmuter(self.transmuter.clone())
                .interrupt(self.workflows.interrupt.clone())
                .history(self.data.session.history.clone())
                .errors(self.errors.clone())
                .scratch(Some(self.data.session.scratch.clone()))
                .streaming(self.data.prefs.view(|s| s.streaming))
                .models(Arc::new(model_map));

            let run_ctx = if let Some(run_cast) = &self.run_cast {
                run_ctx.run_events(run_cast.clone()).build()
            } else {
                run_ctx.build()
            };

            let run_count = self.run_count;
            let errors = self.errors.clone();
            errors.discard();
            let extra_content = self
                .images
                .load_full()
                .iter()
                .filter_map(|image| {
                    let image = rig::message::Image {
                        data: rig::message::DocumentSourceKind::Url(image.into()),
                        ..Default::default()
                    };

                    errors
                        .distil({
                            ImageResolver::builder()
                                .allow_local(run_count == 0)
                                .build()
                                .preprocess(&image)
                        })
                        .map(|image| rig::message::UserContent::Image(image.into_owned()))
                })
                .collect_vec();

            let inputs = RootContext::builder()
                .history(self.data.session.history.clone())
                .workflow(self.workflows.shadow.clone())
                .user_prompt(prompt)
                .extra_content(extra_content)
                .model("default".into())
                .temperature(self.data.prefs.view(|s| s.temperature.into_inner()))
                .build()
                .inputs()
                .unwrap();

            self.workflows.interrupt.store(false, Ordering::Relaxed);

            let mut exec = WorkflowRunner::builder()
                .inputs(inputs)
                .run_ctx(run_ctx)
                .state_view(self.workflows.node_state.view(exec_id))
                .build();

            exec.init(&self.workflows.shadow.graph);

            exec
        };

        let session = self.data.session.clone();
        let running = self.workflows.running.clone();
        let errors = self.errors.clone();
        let interrupt = self.workflows.interrupt.clone();
        let outputs: Arc<ArcSwap<im::OrdMap<String, crate::workflow::Value>>> = Default::default();
        let duration: Arc<ArcSwap<Duration>> = Default::default();
        let started = chrono::offset::Local::now();

        let _ = tx.send(WorkflowEvent::Exec(ExecEvent::Start(
            exec_id,
            self.workflows.editing.clone(),
            started,
        )));
        let entry = WorkflowRun::builder()
            .started(started)
            .duration(duration.clone())
            .workflow(self.workflows.editing.clone())
            .outputs(outputs.clone())
            .build();

        let runs = &mut self.workflows.outputs;
        runs.push_back(entry);
        if runs.len() > 128 {
            *runs = runs.skip(runs.len() - 128);
        }

        let tx = tx.clone();

        thread::spawn(move || {
            let started = SystemTime::now();
            running.store(true, std::sync::atomic::Ordering::Relaxed);
            defer! {
                task_count_.fetch_sub(1, Ordering::Relaxed);
                running.store(false, std::sync::atomic::Ordering::Relaxed);
                notify.notify_one();
            };

            loop {
                if interrupt.load(Ordering::Relaxed) {
                    break;
                }

                let dur = started.elapsed().unwrap_or_default();
                let _ = tx.send(WorkflowEvent::Exec(ExecEvent::Tick(
                    exec_id,
                    dur.as_millis() as u64,
                )));
                duration.store(Arc::new(dur));
                match exec.step(&mut target) {
                    Ok(false) => {
                        errors.distil(exec.root_finish().map_err(|e| e.into()));
                        break;
                    }
                    Ok(true) => {}
                    Err(err) => {
                        errors.push(err.into());
                        break;
                    }
                }

                let rx = exec.run_ctx.outputs.receiver();
                while !rx.is_empty() {
                    let Ok((label, value)) = rx.recv() else {
                        break;
                    };

                    let _ = tx.send(WorkflowEvent::Exec(ExecEvent::Output(
                        exec_id,
                        label.clone(),
                        value.clone(),
                    )));
                    outputs.rcu(|it| it.update(label.clone(), value.clone()));
                }
            }

            let dur = started.elapsed().unwrap_or_default();
            duration.store(Arc::new(dur));
            errors.distil(session.save());

            if errors.load().is_empty()
                && let Some(scratch) = exec.run_ctx.scratch
            {
                scratch.clear();
            }

            let _ = tx.send(WorkflowEvent::Exec(ExecEvent::Finish(
                exec_id,
                dur.as_millis() as u64,
            )));
        });
    }
}
