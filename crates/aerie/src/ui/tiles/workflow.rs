use std::{
    borrow::Cow,
    convert::identity,
    sync::{Arc, atomic::Ordering},
    thread,
    time::{Duration, SystemTime},
};

use arc_swap::ArcSwap;
use egui::{Align2, ComboBox, KeyboardShortcut, RichText};
use egui_extras::{Size, StripBuilder};
use egui_phosphor::regular::{
    ARROW_CLOCKWISE, ARROW_COUNTER_CLOCKWISE, DOWNLOAD_SIMPLE, INFO, MAGIC_WAND, PENCIL, PLAY,
    STOP, TRASH, UPLOAD_SIMPLE,
};
use egui_snarl::ui::SnarlWidget;
use itertools::Itertools;

use crate::{
    config::ConfigExt as _,
    ui::workflow::get_snarl_style,
    utils::ErrorDistiller as _,
    workflow::{
        RootContext, RunContext,
        runner::{WorkflowRun, WorkflowRunner},
    },
};

const SHORTCUT_RUN: KeyboardShortcut = KeyboardShortcut {
    modifiers: egui::Modifiers::CTRL,
    logical_key: egui::Key::Enter,
};

impl super::AppState {
    pub fn workflow_ui(&mut self, ui: &mut egui::Ui) {
        let running = self
            .workflows
            .running
            .load(std::sync::atomic::Ordering::Relaxed);

        if !running && ui.ctx().input_mut(|i| i.consume_shortcut(&SHORTCUT_RUN)) {
            self.exec_workflow();
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Forces new widget state in children after switching or undos so that
            // Snarl will draw our persisted positions and sizes.
            let mut snarl = self.workflows.view_stack.root_snarl().unwrap();
            let frozen = self.workflows.frozen;

            let mut shadow = {
                let shadow = self.workflows.view_stack.leaf();
                let viewer = self.workflow_viewer();
                // Needed for preserving changes by events, but is there a better way?
                viewer.shadow = shadow;

                let widget = SnarlWidget::new()
                    .id(viewer.view_id)
                    .style(get_snarl_style());
                widget.show(&mut snarl, viewer, ui);

                // Unfortunately, there's no event for node movement so we have to
                // iterate through the whole collection to find moved nodes.
                viewer.cast_positions(&snarl);

                // TODO: only when inside canvas
                viewer.handle_copy(ui, widget);

                if !frozen {
                    viewer.handle_paste(&mut snarl, ui, widget);
                }

                viewer.shadow.clone()
            };

            egui::Window::new(INFO)
                .title_bar(false)
                .constrain_to(ui.max_rect())
                .pivot(Align2::LEFT_BOTTOM)
                .default_pos(egui::pos2(16.0, ui.min_rect().max.y))
                .default_size(egui::vec2(200.0, 100.0))
                .movable(true)
                .show(ui.ctx(), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.selectable_value(&mut self.workflows.meta_edit, 0, "Description");
                        ui.selectable_value(&mut self.workflows.meta_edit, 1, "Schema");
                    });

                    let size = ui.available_size();

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if self.workflows.meta_edit == 0 {
                            let mut description =
                                Cow::Borrowed(self.workflows.shadow.description.as_str());

                            ui.add_sized(
                                size,
                                egui::TextEdit::multiline(&mut description)
                                    .hint_text("Add a description for this workflow"),
                            );

                            if let Cow::Owned(desc) = description {
                                shadow = shadow.with_description(&desc);
                            }
                        } else {
                            let mut schema = Cow::Borrowed(self.workflows.shadow.schema.as_str());

                            ui.add_sized(
                                size,
                                egui::TextEdit::multiline(&mut schema)
                                    .hint_text("Add a schema for this workflow"),
                            );

                            if let Cow::Owned(schema) = schema {
                                shadow = shadow.with_schema(&schema);
                            }
                        }
                    });
                });

            self.workflows
                .view_stack
                .propagate(shadow, identity)
                .unwrap();

            egui::Area::new(egui::Id::new("workflow controls"))
                .default_pos(egui::pos2(16.0, 32.0))
                .default_size(egui::vec2(100.0, 100.0))
                .constrain_to(ui.max_rect())
                .movable(true)
                .show(ui.ctx(), |ui| {
                    egui::Frame::dark_canvas(&Default::default())
                        .inner_margin(8.0)
                        .outer_margin(4.0)
                        .corner_radius(8)
                        .show(ui, |ui| {
                            self.workflow_controls(ui);
                        });
                });
        });
    }

    pub fn workflow_controls(&mut self, ui: &mut egui::Ui) {
        let settings = self.settings.clone();
        let errors = self.errors.clone();
        let running = self
            .workflows
            .running
            .load(std::sync::atomic::Ordering::Relaxed);

        ui.set_max_width(150.0);
        ui.vertical_centered_justified(|ui| {
            ui.label("Workflow:");
            ComboBox::from_id_salt("workflow")
                .wrap()
                .width(ui.available_width())
                .selected_text(&self.workflows.editing)
                .show_ui(ui, |ui| {
                    let original = self.workflows.editing.clone();
                    let mut current = &original;

                    let blank = String::new();
                    ui.selectable_value(&mut current, &blank, "");

                    let names = self.workflows.names().map(|s| s.to_string()).collect_vec();
                    for name in &names {
                        ui.selectable_value(&mut current, name, name);
                    }
                    if current != &original {
                        if settings.view(|s| s.autosave) {
                            self.workflows.save();
                        }

                        self.workflows.switch(current);
                    }
                });

            if let Some(renaming) = self.workflows.renaming.as_mut() {
                let editor = ui.text_edit_singleline(renaming);
                if editor.lost_focus() {
                    errors.distil(self.workflows.rename());
                }

                editor.request_focus();
            }

            StripBuilder::new(ui)
                .sizes(Size::exact(16.0), 2)
                .vertical(|mut strip| {
                    strip.cell(|ui| {
                        StripBuilder::new(ui)
                            .sizes(Size::remainder(), 3)
                            .horizontal(|mut strip| {
                                strip.cell(|ui| {
                                    if ui.button(MAGIC_WAND).on_hover_text("Create").clicked() {
                                        let datetime = chrono::offset::Local::now();
                                        let timestamp = datetime
                                            .format("unnamed-%Y-%m-%dT%H:%M:%S")
                                            .to_string();
                                        self.workflows.switch(&timestamp);
                                    }
                                });
                                strip.cell(|ui| {
                                    if ui.button(PENCIL).on_hover_text("Rename").clicked() {
                                        self.workflows.renaming =
                                            Some(self.workflows.editing.clone());
                                    }
                                });
                                strip.cell(|ui| {
                                    ui.menu_button(TRASH, |ui| {
                                        if ui.button("OK").clicked() {
                                            errors.distil(self.workflows.remove());
                                            let next_name = self
                                                .workflows
                                                .names()
                                                .next()
                                                .map(|s| s.to_string())
                                                .unwrap_or("default".to_string());
                                            self.workflows.switch(&next_name);
                                            ui.close();
                                        }
                                    })
                                    .response
                                    .on_hover_text("Delete");
                                });
                            });
                    });
                    strip.cell(|ui| {
                        StripBuilder::new(ui)
                            .sizes(Size::remainder(), 2)
                            .horizontal(|mut strip| {
                                strip.cell(|ui| {
                                    ui.add_enabled_ui(!running, |ui| {
                                        if ui
                                            .button(DOWNLOAD_SIMPLE)
                                            .on_hover_text("Import")
                                            .clicked()
                                            && let Some(path) = rfd::FileDialog::new()
                                                .set_directory(
                                                    settings.view(|s| s.last_export_dir.clone()),
                                                )
                                                .pick_file()
                                        {
                                            settings.update(|s| {
                                                s.last_export_dir = path
                                                    .parent()
                                                    .map(|p| p.to_path_buf())
                                                    .unwrap_or_default()
                                            });
                                            errors.distil(self.workflows.import(&path));
                                        }
                                    });
                                });
                                strip.cell(|ui| {
                                    if ui.button(UPLOAD_SIMPLE).on_hover_text("Export").clicked()
                                        && let Some(path) = rfd::FileDialog::new()
                                            .set_directory(
                                                settings.view(|s| s.last_export_dir.clone()),
                                            )
                                            .set_file_name(format!(
                                                "{}.yml",
                                                self.workflows.editing
                                            ))
                                            .save_file()
                                    {
                                        settings.update(|s| {
                                            s.last_export_dir = path
                                                .parent()
                                                .map(|p| p.to_path_buf())
                                                .unwrap_or_default()
                                        });
                                        errors.distil(self.workflows.export(&path));
                                    }
                                });
                            });
                    });
                });

            ui.separator();

            // not loving the boilerplate but this gets the right results
            // Maybe should put the whole panel into the vertical strip
            StripBuilder::new(ui)
                .size(Size::exact(16.0))
                .vertical(|mut strip| {
                    strip.cell(|ui| {
                        StripBuilder::new(ui)
                            .sizes(Size::remainder(), 2)
                            .horizontal(|mut strip| {
                                strip.cell(|ui| {
                                    let stack = self.workflows.get_undo_count();
                                    ui.add_enabled_ui(!running && stack > 0, |ui| {
                                        if ui
                                            .button(ARROW_COUNTER_CLOCKWISE)
                                            .on_hover_text(format!("{stack}"))
                                            .clicked()
                                        {
                                            self.workflows.undo();
                                        }
                                    });
                                });
                                strip.cell(|ui| {
                                    let stack = self.workflows.get_redo_count();
                                    ui.add_enabled_ui(!running && stack > 0, |ui| {
                                        if ui
                                            .button(ARROW_CLOCKWISE)
                                            .on_hover_text(format!("{stack}"))
                                            .clicked()
                                        {
                                            self.workflows.redo();
                                        }
                                    });
                                });
                            });
                    });
                });

            if !settings.view(|s| s.autosave) {
                ui.add_enabled_ui(self.workflows.has_changes(), |ui| {
                    if ui.button("Save").clicked() {
                        self.workflows.save();
                    }
                });
            } else if !self.workflows.frozen
                && self.workflows.has_changes()
                && self.workflows.modtime.elapsed().unwrap_or(Duration::ZERO)
                    > Duration::from_secs(2)
            {
                self.workflows.save();
            }

            ui.separator();

            ui.scope(|ui| {
                ui.style_mut().spacing.button_padding.y = 8.0;

                let (frozen_label, frozen_hint) = if running {
                    ("« running »", "Please wait...")
                } else if self.workflows.frozen {
                    ("« frozen »", "Click to re-enable editing.")
                } else {
                    ("« editing »", "Click to prevent new changes.")
                };

                ui.toggle_value(&mut self.workflows.frozen, frozen_label)
                    .on_hover_text(frozen_hint);
            });

            ui.separator();
            ui.scope(|ui| {
                // Bigger button
                ui.style_mut().spacing.button_padding.y = 16.0;
                if running {
                    let interrupting = self.workflows.interrupt.load(Ordering::Relaxed);
                    ui.add_enabled_ui(!interrupting, |ui| {
                        if ui.button(stop_layout(interrupting)).clicked() {
                            self.workflows.interrupt.store(true, Ordering::Relaxed);
                        }
                    });
                } else if ui.button(play_layout()).clicked() {
                    self.run_count = 0;
                    self.exec_workflow();
                }
            });

            // TODO: pause and cancel buttons
        });
    }

    // TODO: decouple editing and execution workflows
    /// Runs the workflow currently being edited and updates nodes in the viewer with results.
    pub fn exec_workflow(&mut self) {
        let mut target = self.workflows.view_stack.root_snarl().unwrap();
        let task_count_ = self.task_count.clone();

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
                .user_prompt(self.prompt.clone())
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

fn play_layout() -> egui::text::LayoutJob {
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

fn stop_layout(stopping: bool) -> egui::text::LayoutJob {
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
