use eframe::egui;
use itertools::Itertools;
use std::collections::BTreeSet;

use minijinja::{Environment, context};
use rig::{agent::PromptRequest, message::Message};
use std::{borrow::Cow, collections::VecDeque, sync::atomic::Ordering, time::Instant};
use tracing::Level;

use crate::{
    ChatContent, LogEntry, Pipeline, Workstep,
    ui::toggled_field,
    utils::{CowExt as _, ErrorDistiller as _},
};

impl super::AppState {
    pub fn pipeline_ui(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let mut settings_rw = self.settings.write().unwrap();

            if ui.button("+ New").clicked() {
                let existing = settings_rw
                    .pipelines
                    .iter()
                    .map(|it| it.name.clone())
                    .collect::<BTreeSet<_>>();

                let mut counter = 0;
                let name = loop {
                    let name = format!("New pipeline {counter:04}");
                    if !existing.contains(&name) {
                        break name;
                    }
                    counter += 1;
                };

                settings_rw.automation = Some(name.clone());
                settings_rw.pipelines.push(Pipeline {
                    name,
                    ..Default::default()
                });
            }

            let toolsets = settings_rw.tools.toolset.keys().cloned().collect_vec();

            ui.add_enabled_ui(settings_rw.automation.is_some(), |ui| {
                let pipeline_name = settings_rw.automation.to_owned().unwrap_or_default();
                if let Some(pipeline) = settings_rw
                    .pipelines
                    .iter_mut()
                    .find(|it| it.name == pipeline_name)
                {
                    let mut name_changed = false;
                    let mut checked = pipeline.preamble.is_some();
                    let mut value = pipeline.preamble.to_owned().unwrap_or_default();

                    egui::Grid::new("pipeline settings")
                        .num_columns(2)
                        .striped(true)
                        .show(ui, |ui| {
                            ui.label("Name").on_hover_text("Name of the pipeline");
                            name_changed =
                                ui.text_edit_singleline(&mut pipeline.name).changed();
                            ui.end_row();

                            ui.label("Preamble").on_hover_text("Optionally, override the system preamble.\nIf enabled and empty, then no preamble is used.");
                            ui.checkbox(&mut checked, "Override");
                            ui.end_row();
                        });

                    if checked {
                        ui.add(
                            egui::TextEdit::multiline(&mut value)
                                .hint_text("pipeline specific preamble"),
                        );
                    }
                    // ui.add_visible(checked, egui::TextEdit::multiline(&mut value));

                    pipeline.preamble = if checked { Some(value) } else { None };

                    ui.separator();
                    ui.heading("Steps");
                    for (i, step) in pipeline.steps.iter_mut().enumerate() {
                        egui::Frame::new()
                            .stroke(egui::Stroke::new(1.0, egui::Color32::GRAY))
                            .corner_radius(4)
                            .outer_margin(4)
                            .inner_margin(8)
                            .show(ui, |ui| {
                                let id = ui.id().with(i);
                                egui::Grid::new(id).num_columns(2).striped(true).show(
                                    ui,
                                    |ui| {
                                        ui.label("Skip").on_hover_text("Disable this step and advance to the next");
                                        ui.checkbox(&mut step.disabled, "");
                                        ui.end_row();

                                        toggled_field(
                                            ui,
                                            "Temperature",
                                            None::<String>,
                                            &mut step.temperature,
                                            |ui, value| {
                                                ui.add(egui::Slider::new(value, 0.0..=1.0));
                                            },
                                        );
                                        ui.end_row();

                                        toggled_field(
                                            ui,
                                            "Depth",
                                            None::<String>,
                                            &mut step.depth,
                                            |ui, value| {
                                                ui.add(egui::Slider::new(value, 0..=100));
                                            },
                                        );
                                        ui.end_row();

                                        toggled_field(
                                            ui,
                                            "Preamble",
                                            None::<String>,
                                            &mut step.preamble,
                                            |ui, value| {
                                                ui.add(
                                                    egui::TextEdit::multiline(value)
                                                        .hint_text("Step specific preamble"),
                                                );
                                            },
                                        );
                                        ui.end_row();

                                        ui.label("Prompt");
                                        ui.text_edit_multiline(&mut step.prompt);
                                        ui.end_row();

                                        toggled_field(
                                            ui,
                                            "Tools",
                                            None::<String>,
                                            &mut step.tools,
                                            |ui, value| {

                                                egui::ComboBox::from_id_salt("Tools")
                                                    .selected_text(value.as_str()).show_ui(ui, |ui| {
                                                        for name in  &toolsets {
                                                            ui.selectable_value(value, name.clone(), name);
                                                        }
                                                    });

                                            },
                                        );
                                    },
                                );
                            });
                    }
                    if ui.button("+ New step").clicked() {
                        pipeline.steps.push(Workstep::default());
                    }

                    if name_changed {
                        settings_rw.automation = Some(pipeline.name.clone());
                    }
                }
            });
        });
    }

    pub fn exec_pipeline(&mut self, pipeline: Pipeline, user_prompt: String) {
        let session_ = self.session.clone();
        let scratch_ = self.session.scratch.clone();
        let task_count_ = self.task_count.clone();
        let agent_factory_ = self.agent_factory.clone();
        let log_history_ = self.log_history.clone();
        let branch = self.session.view(|hist| hist.head.clone());
        let errors = self.errors.clone();

        self.rt.spawn(async move {
            task_count_.fetch_add(1, Ordering::Relaxed);
            let start = Instant::now();
            let env = Environment::new();

            for workstep in pipeline.steps {
                if workstep.disabled {
                    continue;
                }

                let Ok(tmpl) = env.template_from_str(&workstep.prompt) else {
                    tracing::warn!("Cannot load template from step: {workstep:?}");
                    continue;
                };

                let Ok(prompt) = tmpl.render(context! {user_prompt}) else {
                    tracing::warn!("Cannot render prompt for step: {workstep:?}");
                    continue;
                };

                let mut history = session_.view(|session| {
                    let mut buffer: VecDeque<&Message> = if let Some(depth) = workstep.depth {
                        VecDeque::with_capacity(depth)
                    } else {
                        VecDeque::new()
                    };

                    for entry in session.iter() {
                        match &entry.content {
                            ChatContent::Message(message) => buffer.push_back(message),
                            ChatContent::Aside { content, .. } => {
                                if let Some(message) = content.last() {
                                    buffer.push_back(message);
                                }
                            }
                            ChatContent::Error(_) => {}
                        }
                    }

                    let mut scratch = scratch_.write().unwrap();
                    scratch.push(Ok(Message::user(&prompt)));

                    for message in scratch[..scratch.len() - 1]
                        .iter()
                        .filter_map(|it| it.as_ref().ok())
                    {
                        buffer.push_back(message);
                    }

                    // Laziness avoids cloning older items we won't use
                    buffer.into_iter().cloned().collect::<Vec<_>>()
                });

                let agent = agent_factory_.agent(&workstep);
                let request = PromptRequest::new(&agent, &prompt)
                    .multi_turn(5)
                    .with_history(&mut history);

                match request.await {
                    Ok(response) => {
                        let mut chat = scratch_.write().unwrap();
                        chat.push(Ok(Message::assistant(response)));
                    }

                    Err(err) => {
                        tracing::warn!("Failed chat: {err:?}");

                        let mut chat = scratch_.write().unwrap();
                        let err_str = format!("{err}");
                        chat.push(Err(err_str.clone()));

                        let mut log_rw = log_history_.write().unwrap();
                        log_rw.push(LogEntry(Level::ERROR, format!("Error: {err:?}")));
                    }
                }
            }

            let scratch = std::mem::take(&mut *scratch_.write().unwrap());
            errors.distil(session_.transform(|history| {
                let history = Cow::Borrowed(history);
                if scratch.len() <= 2 {
                    history.try_moo(|h| {
                        h.extend(scratch.into_iter().map(|it| it.into()), Some(&branch))
                    })
                } else {
                    use itertools::{Either, Itertools as _};

                    // Split scratch so messages can be collapsed but errors will appear inline
                    let mut iter = scratch.into_iter();
                    let Some(prompt_msg) = iter.next() else {
                        unreachable!()
                    };

                    let (content, errors): (Vec<_>, Vec<_>) = iter.partition_map(|r| match r {
                        Ok(v) => Either::Left(v),
                        Err(v) => Either::Right(v),
                    });

                    let aside_msgs = ChatContent::Aside {
                        automation: pipeline.name,
                        prompt: user_prompt,
                        collapsed: true,
                        content,
                    };

                    let error_msgs = errors.into_iter().map(ChatContent::Error);

                    // No inconsistent partial updates if intermediate ops fail
                    history
                        .try_moo(|h| h.push(prompt_msg.into(), Some(&branch)))?
                        .try_moo(|h| h.push(aside_msgs, Some(&branch)))?
                        .try_moo(|h| h.extend(error_msgs, Some(&branch)))
                }
            }));

            task_count_.fetch_sub(1, Ordering::Relaxed);
            tracing::info!(
                "Request completed in {} seconds.",
                Instant::now().duration_since(start).as_secs_f32()
            );
        });
    }
}
