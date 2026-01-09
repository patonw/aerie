use std::{
    sync::{Arc, atomic::Ordering},
    thread,
    time::{Duration, SystemTime},
};

use arc_swap::ArcSwap;
use egui::RichText;
use egui_phosphor::regular::{PLAY, STOP};

use crate::{
    config::ConfigExt as _,
    utils::ErrorDistiller as _,
    workflow::{
        RootContext, RunContext,
        runner::{WorkflowRun, WorkflowRunner},
    },
};

impl super::AppState {
    /// Runs the workflow currently being edited and updates nodes in the viewer with results.
    pub fn exec_workflow(&mut self) {
        let mut target = self.workflows.view_stack.root_snarl().unwrap();
        let task_count_ = self.task_count.clone();

        let prompt = self.prompt.clone();

        self.settings
            .update(|s| s.automation = Some(self.workflows.editing.clone()));

        self.session.scratch.clear();

        let mut exec = {
            let run_ctx = RunContext::builder()
                .runtime(self.rt.clone())
                .agent_factory(self.agent_factory.clone())
                .node_state(self.workflows.node_state.clone())
                .previews(self.workflows.previews.clone())
                .transmuter(self.transmuter.clone())
                .interrupt(self.workflows.interrupt.clone())
                .history(self.session.history.clone())
                .seed(self.settings.view(|s| s.seed.clone()))
                .errors(self.errors.clone())
                .scratch(Some(self.session.scratch.clone()))
                .streaming(self.settings.view(|s| s.streaming))
                .build();

            let inputs = RootContext::builder()
                .history(self.session.history.clone())
                .graph(self.workflows.shadow.clone())
                .user_prompt(prompt)
                .model(self.settings.view(|s| s.llm_model.clone()))
                .temperature(self.settings.view(|s| s.temperature))
                .build()
                .inputs()
                .unwrap();

            self.workflows.interrupt.store(false, Ordering::Relaxed);

            let mut exec = WorkflowRunner::builder()
                .inputs(inputs)
                .run_ctx(run_ctx)
                .state_view(self.workflows.node_state.view(&self.workflows.shadow.uuid))
                .build();

            exec.init(&self.workflows.shadow);

            exec
        };

        let session = self.session.clone();
        let running = self.workflows.running.clone();
        let errors = self.errors.clone();
        let interrupt = self.workflows.interrupt.clone();
        let outputs: Arc<ArcSwap<im::OrdMap<String, crate::workflow::Value>>> = Default::default();
        let duration: Arc<ArcSwap<Duration>> = Default::default();
        let started = chrono::offset::Local::now();

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

        thread::spawn(move || {
            let started = SystemTime::now();
            task_count_.fetch_add(1, Ordering::Relaxed);
            running.store(true, std::sync::atomic::Ordering::Relaxed);

            loop {
                if interrupt.load(Ordering::Relaxed) {
                    break;
                }

                duration.store(Arc::new(started.elapsed().unwrap_or_default()));
                match exec.step(&mut target) {
                    Ok(false) => {
                        exec.root_finish().unwrap();
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
                    tracing::info!("Received output {label}: {value:?}");

                    outputs.rcu(|it| it.update(label.clone(), value.clone()));
                }
            }

            duration.store(Arc::new(started.elapsed().unwrap_or_default()));
            errors.distil(session.save());
            running.store(false, std::sync::atomic::Ordering::Relaxed);
            task_count_.fetch_sub(1, Ordering::Relaxed);

            if errors.load().is_empty()
                && let Some(scratch) = exec.run_ctx.scratch
            {
                scratch.clear();
            }
        });
    }
}

pub fn play_button() -> egui::Button<'static> {
    egui::Button::new(play_layout())
}

pub fn play_layout() -> egui::text::LayoutJob {
    use egui::{Align, FontSelection, Style, text::LayoutJob};

    let style = Style::default();
    let mut layout_job = LayoutJob::default();
    RichText::new(PLAY)
        .color(egui::Color32::GREEN)
        .strong()
        .heading()
        .append_to(
            &mut layout_job,
            &style,
            FontSelection::Default,
            Align::Center,
        );

    RichText::new(" Run")
        .color(style.visuals.text_color())
        .append_to(
            &mut layout_job,
            &style,
            FontSelection::Default,
            Align::Center,
        );

    layout_job
}

pub fn stop_button(stopping: bool) -> egui::Button<'static> {
    egui::Button::new(stop_layout(stopping))
}

pub fn stop_layout(stopping: bool) -> egui::text::LayoutJob {
    use egui::{Align, FontSelection, Style, text::LayoutJob};

    let style = Style::default();
    let mut layout_job = LayoutJob::default();
    RichText::new(STOP)
        .color(egui::Color32::RED)
        .strong()
        .heading()
        .append_to(
            &mut layout_job,
            &style,
            FontSelection::Default,
            Align::Center,
        );

    RichText::new(if stopping { " Stopping" } else { " Stop" })
        .color(style.visuals.text_color())
        .append_to(
            &mut layout_job,
            &style,
            FontSelection::Default,
            Align::Center,
        );

    layout_job
}
