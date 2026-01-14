use std::{
    collections::{BTreeMap, BTreeSet},
    iter,
    sync::Arc,
};

use anyhow::Context as _;
use cached::proc_macro::cached;
use egui::{Color32, Hyperlink, RichText, Sense, Ui, emath::TSTransform};
use egui_phosphor::regular::{CHECK_CIRCLE, HAND_PALM, HOURGLASS_MEDIUM, PLAY_CIRCLE, WARNING};
use egui_snarl::{
    InPinId, NodeId, OutPinId, Snarl,
    ui::{SnarlStyle, SnarlViewer, SnarlWidget, get_selected_nodes},
};
use im::vector;
use typed_builder::TypedBuilder;

use crate::{
    ui::shortcuts::{ShortcutHandler, squelch},
    utils::ErrorDistiller as _,
    workflow::{
        EditContext, GraphId, MetaNode, ShadowGraph, WorkNode,
        nodes::{CommentNode, Subgraph},
        runner::{ExecState, NodeStateMap},
    },
};

use super::AppEvents;

#[cached]
pub fn get_snarl_style() -> SnarlStyle {
    use egui_snarl::ui::{BackgroundPattern, Grid, NodeLayout, PinPlacement, SnarlStyle};
    SnarlStyle {
        crisp_magnified_text: Some(true),
        bg_pattern: Some(BackgroundPattern::Grid(Grid::new(
            egui::Vec2::new(100.0, 100.0),
            0.0,
        ))),
        node_frame: SnarlStyle::default()
            .node_frame
            .map(|frame| frame.inner_margin(16.0)),
        node_layout: Some(NodeLayout::sandwich()),
        pin_placement: Some(PinPlacement::Edge),
        ..Default::default()
    }
}

#[derive(Clone, TypedBuilder)]
pub struct ViewStack {
    pub root_id: egui::Id,

    pub path: im::Vector<NodeId>,

    pub levels: im::Vector<ShadowGraph<WorkNode>>,
}

impl ViewStack {
    pub fn new(root_graph: ShadowGraph<WorkNode>, path: impl Iterator<Item = NodeId>) -> Self {
        let mut me = Self {
            root_id: egui::Id::new(root_graph.uuid),
            path: Default::default(),
            levels: vector![root_graph],
        };

        for id in path {
            if me.enter(id).is_err() {
                break;
            }
        }

        me
    }

    pub fn from_root(root_graph: ShadowGraph<WorkNode>) -> Self {
        Self::new(root_graph, iter::empty())
    }

    /// Replace root graph with a different version.
    ///
    /// Attempts to preserve path, but will navigate as far as possible
    /// if subgraphs are absent.
    pub fn switch(&mut self, root_graph: ShadowGraph<WorkNode>) {
        let path = self.path.clone();
        *self = Self::new(root_graph, path.into_iter());
    }

    pub fn is_empty(&self) -> bool {
        self.path.is_empty()
    }

    pub fn names(&'_ self) -> impl DoubleEndedIterator<Item = String> {
        self.path
            .iter()
            .zip(self.levels.skip(1))
            .map(|(node_id, graph)| {
                graph
                    .nodes
                    .get(node_id)
                    .map(|m| m.value.as_ui().title().to_string())
                    .unwrap_or("???".to_string())
            })
            .chain(std::iter::once("(root)".to_string()))
    }

    pub fn root(&self) -> ShadowGraph<WorkNode> {
        assert!(!self.levels.is_empty());
        self.levels.back().cloned().unwrap()
    }

    pub fn root_snarl(&self) -> anyhow::Result<Snarl<WorkNode>> {
        egui_snarl::Snarl::try_from(self.root())
    }

    pub fn leaf(&self) -> ShadowGraph<WorkNode> {
        assert!(!self.levels.is_empty());
        self.levels.front().cloned().unwrap()
    }

    pub fn leaf_snarl(&self) -> anyhow::Result<Snarl<WorkNode>> {
        egui_snarl::Snarl::try_from(self.leaf())
    }

    /// The id of the parent graph and node id of the subgraph container
    pub fn parent_id(&self) -> Option<(GraphId, NodeId)> {
        if self.is_empty() {
            None
        } else {
            Some((self.levels[1].uuid, self.path[0]))
        }
    }

    pub fn view_id(&self) -> egui::Id {
        self.root_id.with(&self.path)
    }

    pub fn exit(&mut self, levels: usize) -> anyhow::Result<()> {
        for _ in 0..levels {
            if self.path.pop_front().is_none() || self.levels.pop_front().is_none() {
                anyhow::bail!("stack is empty")
            }
        }

        Ok(())
    }

    pub fn enter(&mut self, node: NodeId) -> anyhow::Result<()> {
        let parent = self
            .leaf()
            .nodes
            .get(&node)
            .cloned()
            .context("No such node in parent graph")?;

        let graph = parent.value;
        if !graph.is_subgraph() {
            anyhow::bail!("Not a subgraph");
        }

        let WorkNode::Subgraph(subgraph) = graph else {
            anyhow::bail!("Could not extract subgraph");
        };

        self.path.push_front(node);
        self.levels.push_front(subgraph.graph.clone());

        Ok(())
    }

    /// Cascades changes in the subgraphs up to the root,
    /// transforming each level with the callback.
    pub fn propagate<F>(&mut self, shadow: ShadowGraph<WorkNode>, f: F) -> anyhow::Result<()>
    where
        F: Fn(ShadowGraph<WorkNode>) -> ShadowGraph<WorkNode>,
    {
        let mut ids = self.path.iter();
        let mut graphs = self.levels.iter_mut();

        let Some(child_graph) = graphs.next() else {
            unreachable!()
        };

        // Transform the new leaf before assigning it to the stack
        let shadow = f(shadow);
        if !child_graph.fast_eq(&shadow) {
            *child_graph = shadow.clone();
        }

        let mut child_graph = shadow;

        let Some(mut child_id) = ids.next() else {
            return Ok(());
        };

        loop {
            let Some(parent_graph) = graphs.next() else {
                unreachable!()
            };

            let mut target = parent_graph.clone();

            match target.nodes.get(child_id) {
                Some(meta) if !meta.value.is_subgraph() => anyhow::bail!(
                    "Child node {child_id:?} of {:?} is not a subgraph",
                    parent_graph.uuid
                ),
                Some(meta) => match &meta.value {
                    WorkNode::Subgraph(node) => {
                        if !node.graph.fast_eq(&child_graph) {
                            // Replace the subgraph in the parent's node
                            let node = Subgraph {
                                graph: child_graph.clone(),
                                ..node.clone()
                            };

                            let meta = MetaNode {
                                value: WorkNode::Subgraph(node),
                                ..meta.clone()
                            };

                            target.nodes.insert(*child_id, meta);
                        }

                        // Transform and save the parent graph
                        let target = f(target);
                        if !parent_graph.fast_eq(&target) {
                            *parent_graph = target.clone();
                        }
                        child_graph = target;
                    }
                    _ => unreachable!(),
                },
                None => anyhow::bail!(
                    "No such subgraph {child_id:?} in parent graph {:?}",
                    parent_graph.uuid
                ),
            }

            if let Some(parent_id) = ids.next() {
                child_id = parent_id;
            } else {
                break;
            }
        }

        Ok(())
    }
}

#[derive(Clone, TypedBuilder)]
pub struct WorkflowViewer {
    #[builder(default = egui::Id::new("default_viewer"))]
    pub view_id: egui::Id,

    #[builder(default)]
    pub transform: TSTransform,

    pub edit_ctx: EditContext,

    pub events: Arc<AppEvents>,

    #[builder(default)]
    pub shadow: ShadowGraph<WorkNode>,

    #[builder(default)]
    pub node_state: NodeStateMap,

    #[builder(default)]
    pub running: bool,

    #[builder(default)]
    pub frozen: bool,

    #[builder(default)]
    pub rename_node: Option<NodeId>,
}

impl WorkflowViewer {
    pub fn frozen(&self) -> bool {
        self.running || self.frozen
    }

    pub fn can_edit(&self) -> bool {
        !self.frozen()
    }

    pub fn cast_positions(&mut self, snarl: &Snarl<WorkNode>) {
        if self.frozen {
            return;
        }

        let mut nodes = self.shadow.nodes.clone();
        for (id, pos, data) in snarl.nodes_pos_ids() {
            nodes
                .entry(id)
                .and_modify(|n| n.pos = pos)
                .or_insert(MetaNode {
                    value: data.clone(),
                    pos,
                    open: true,
                });
        }

        if nodes != self.shadow.nodes {
            self.shadow = ShadowGraph {
                nodes,
                ..self.shadow.clone()
            };
        }
    }

    pub fn handle_copy(&self, ui: &mut Ui, widget: SnarlWidget) -> Vec<NodeId> {
        let pos = self.transform.inverse()
            * ui.ctx()
                .input(|i| i.pointer.interact_pos())
                .unwrap_or_default();

        let selection = widget.get_selected_nodes(ui);
        if !selection.is_empty() {
            let copied = filter_graph(self.shadow.clone(), pos.to_vec2(), &selection);
            if let Ok(text) = serde_yml::to_string(&copied) {
                ui.ctx().copy_text(text);
            }
        }

        selection
    }

    pub fn handle_paste(
        &mut self,
        snarl: &mut Snarl<WorkNode>,
        ui: &mut egui::Ui,
        widget: SnarlWidget,
    ) {
        if let Some(text) = ui.ctx().input_mut(|input| {
            input.events.iter().find_map(|ev| {
                if let egui::Event::Paste(text) = ev {
                    Some(text.clone())
                } else {
                    None
                }
            })
        }) {
            let pos = self.transform.inverse()
                * ui.ctx()
                    .input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()))
                    .unwrap_or_default();

            if let Ok(shadow) = serde_yml::from_str(&text) {
                let inserted = merge_graphs(snarl, &mut self.shadow, pos.to_vec2(), shadow);
                widget.update_selected_nodes(ui, |nodes| {
                    *nodes = inserted;
                });
            }
        }
    }

    pub fn target_nodes(&mut self, ui: &mut Ui, node: Option<NodeId>) -> Vec<NodeId> {
        let selection = get_selected_nodes(self.view_id, ui.ctx());
        if let Some(node) = node {
            if selection.contains(&node) {
                selection
            } else {
                vec![node]
            }
        } else {
            selection
        }
    }

    pub fn remove_nodes(&mut self, ui: &mut Ui, snarl: &mut Snarl<WorkNode>, node: Option<NodeId>) {
        let targets = self.target_nodes(ui, node);

        for node in &targets {
            if !&snarl[*node].is_protected() {
                snarl.remove_node(*node);
                self.shadow = self.shadow.enable_node(*node).without_node(node);
            }
        }
    }

    pub fn disable_nodes(
        &mut self,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<WorkNode>,
        node: Option<NodeId>,
    ) {
        let targets = self.target_nodes(ui, node);

        if let Some(node) = node
            && self.shadow.is_disabled(node)
        {
            for node in &targets {
                self.shadow = self.shadow.enable_node(*node);
            }
        } else if targets.iter().all(|n| self.shadow.is_disabled(*n)) {
            for node in &targets {
                self.shadow = self.shadow.enable_node(*node);
            }
        } else {
            for node in &targets {
                let can_disable =
                    !matches!(snarl[*node], WorkNode::Comment(_)) && !snarl[*node].is_protected();
                if can_disable {
                    self.shadow = self.shadow.disable_node(*node);
                }
            }
        }
    }
}

impl SnarlViewer<WorkNode> for WorkflowViewer {
    fn title(&mut self, node: &WorkNode) -> String {
        node.as_ui().title().to_string()
    }

    fn node_frame(
        &mut self,
        default: egui::Frame,
        node: NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        snarl: &Snarl<WorkNode>,
    ) -> egui::Frame {
        if matches!(snarl[node], WorkNode::Comment(_)) {
            default.fill(CommentNode::bg_color())
        } else {
            default
        }
    }

    fn header_frame(
        &mut self,
        default: egui::Frame,
        node: NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        snarl: &Snarl<WorkNode>,
    ) -> egui::Frame {
        if matches!(snarl[node], WorkNode::Comment(_)) {
            let node_info = snarl.get_node_info(node).unwrap();
            if node_info.open {
                default.fill(CommentNode::bg_color())
            } else {
                default.fill(Color32::from_rgb(0x88, 0x88, 0))
            }
        } else {
            default
        }
    }

    fn show_header(
        &mut self,
        node: NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<WorkNode>,
    ) {
        let node_state = self.node_state.view(&self.shadow.uuid);

        if matches!(snarl[node], WorkNode::Comment(_)) {
            return;
        }

        egui::Sides::new().show(
            ui,
            |ui| {
                if let Some(title) = snarl[node].as_ui_mut().title_mut() {
                    if self.rename_node == Some(node) {
                        let widget = egui::TextEdit::singleline(title).desired_width(200.0);
                        let resp = squelch(ui.add(widget));

                        if resp.lost_focus() {
                            self.rename_node = None;
                        }

                        resp.request_focus();
                    } else {
                        let widget = egui::Label::new(snarl[node].as_ui().title()).truncate();
                        if ui
                            .add(widget)
                            .interact(egui::Sense::click())
                            .double_clicked()
                        {
                            self.rename_node = Some(node);
                        }
                    }
                } else {
                    let title = snarl[node].as_ui().title();
                    ui.label(title);
                }
            },
            |ui| match node_state.get(&node) {
                Some(ExecState::Waiting(_)) => {
                    ui.label(RichText::new(HOURGLASS_MEDIUM).color(Color32::ORANGE))
                        .on_hover_text("Waiting");
                }
                Some(ExecState::Ready) => {
                    ui.label(RichText::new(PLAY_CIRCLE).color(Color32::BLUE))
                        .on_hover_text("Ready");
                }
                Some(ExecState::Running) => {
                    ui.add(egui::Spinner::new().color(Color32::LIGHT_GREEN))
                        .on_hover_text("Running");
                }
                Some(ExecState::Done(_)) => {
                    ui.label(RichText::new(CHECK_CIRCLE).color(Color32::GREEN))
                        .on_hover_text("Done");
                }
                Some(ExecState::Disabled) => {
                    ui.label(HAND_PALM).on_hover_text("Disabled");
                }
                Some(ExecState::Failed(err)) => {
                    if ui
                        .label(RichText::new(WARNING).color(Color32::RED))
                        .on_hover_text(format!("{err:?}"))
                        .interact(egui::Sense::click())
                        .clicked()
                    {
                        let error = err.clone();
                        self.edit_ctx.errors.push(error.into());
                    }
                }
                None => {}
            },
        );
    }

    fn final_node_rect(
        &mut self,
        node: NodeId,
        rect: egui::Rect,
        ui: &mut Ui,
        snarl: &mut Snarl<WorkNode>,
    ) {
        if self.shadow.is_disabled(node) {
            let painter = ui.painter();
            painter.rect_filled(
                rect,
                16.0,
                egui::Color32::from_rgb(0x42, 0, 0).gamma_multiply(0.5),
            );
        }

        let node_egui_id = self.view_id.with(("snarl-node", node)); //.with("frame");
        let hover_resp = ui.interact(rect, node_egui_id, Sense::hover());
        if hover_resp.hovered() {
            let mut handler = ShortcutHandler::builder().snarl(snarl).viewer(self).build();

            handler.node_shortcuts(ui, node);
        }

        // A bit hacky
        let output_reset = self
            .edit_ctx
            .output_reset
            .swap(Arc::new(Default::default()));
        for out_pin_id in output_reset.iter() {
            let out_pin = snarl.out_pin(*out_pin_id);
            for in_pin_id in &out_pin.remotes {
                let in_pin = snarl.in_pin(*in_pin_id);
                self.disconnect(&out_pin, &in_pin, snarl);
                self.connect(&out_pin, &in_pin, snarl);
            }
        }

        self.shadow = self.shadow.with_node(&node, snarl.get_node_info(node));
    }

    fn has_on_hover_popup(&mut self, node: &WorkNode) -> bool {
        !node.as_ui().tooltip().is_empty()
    }

    fn show_on_hover_popup(
        &mut self,
        node: NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<WorkNode>,
    ) {
        if self.shadow.is_disabled(node) {
            ui.label("Node has been disabled.\n\nThis and downstream nodes will not be executed.");
        } else {
            let tooltip = snarl[node].as_ui().tooltip();
            ui.label(tooltip);
        }
    }

    fn inputs(&mut self, node: &WorkNode) -> usize {
        node.as_ui().inputs()
    }

    fn show_input(
        &mut self,
        pin: &egui_snarl::InPin,
        ui: &mut egui::Ui,
        snarl: &mut egui_snarl::Snarl<WorkNode>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        ui.add_enabled_ui(self.can_edit(), |ui| {
            let value = match &*pin.remotes {
                [] => None,
                [remote, ..] => {
                    let other = snarl[remote.node].as_ui();
                    Some(other.preview(remote.output))
                }
            };

            let node_id = pin.id.node;
            self.edit_ctx.current_node = node_id;
            self.edit_ctx.disabled = self.shadow.is_disabled(node_id);
            let node = &mut snarl[node_id];
            let pin = node
                .as_ui_mut()
                .show_input(ui, &self.edit_ctx, pin.id.input, value);

            self.shadow = self
                .shadow
                .with_node(&node_id, snarl.get_node_info(node_id));

            pin
        })
        .inner
    }

    fn outputs(&mut self, node: &WorkNode) -> usize {
        node.as_ui().outputs()
    }

    fn show_output(
        &mut self,
        pin: &egui_snarl::OutPin,
        ui: &mut egui::Ui,
        snarl: &mut egui_snarl::Snarl<WorkNode>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        ui.add_enabled_ui(self.can_edit(), |ui| {
            let node_id = pin.id.node;
            self.edit_ctx.current_node = node_id;
            self.edit_ctx.disabled = self.shadow.is_disabled(node_id);
            let node = &mut snarl[node_id];
            let pin = node
                .as_ui_mut()
                .show_output(ui, &self.edit_ctx, pin.id.output);

            self.shadow = self
                .shadow
                .with_node(&node_id, snarl.get_node_info(node_id));
            pin
        })
        .inner
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<WorkNode>) -> bool {
        self.can_edit()
    }

    fn show_graph_menu(&mut self, pos: egui::Pos2, ui: &mut Ui, snarl: &mut Snarl<WorkNode>) {
        ui.menu_button("Control", |ui| {
            if ui.button("Fallback").clicked() {
                snarl.insert_node(pos, WorkNode::Fallback(Default::default()));
                ui.close();
            }

            if ui.button("Matcher").clicked() {
                snarl.insert_node(pos, WorkNode::Matcher(Default::default()));
                ui.close();
            }

            if ui.button("Select").clicked() {
                snarl.insert_node(pos, WorkNode::Select(Default::default()));
                ui.close();
            }

            if ui.button("Demote").clicked() {
                snarl.insert_node(pos, WorkNode::Demote(Default::default()));
                ui.close();
            }

            if ui.button("Panic").clicked() {
                snarl.insert_node(pos, WorkNode::Panic(Default::default()));
                ui.close();
            }
        });

        ui.menu_button("Value", |ui| {
            if ui.button("Number").clicked() {
                snarl.insert_node(pos, WorkNode::Number(Default::default()));
                ui.close();
            }

            if ui.button("Plain Text").clicked() {
                snarl.insert_node(pos, WorkNode::Text(Default::default()));
                ui.close();
            }

            if ui.button("Template").clicked() {
                snarl.insert_node(pos, WorkNode::TemplateNode(Default::default()));
                ui.close();
            }
        });

        ui.menu_button("LLM", |ui| {
            if ui.button("Agent").clicked() {
                snarl.insert_node(pos, WorkNode::Agent(Default::default()));
                ui.close();
            }

            if ui.button("Context").clicked() {
                snarl.insert_node(pos, WorkNode::Context(Default::default()));
                ui.close();
            }

            if ui.button("Chat").clicked() {
                snarl.insert_node(pos, WorkNode::Chat(Default::default()));
                ui.close();
            }

            if ui.button("Structured").clicked() {
                snarl.insert_node(pos, WorkNode::Structured(Default::default()));
                ui.close();
            }
        });

        ui.menu_button("Tools", |ui| {
            if ui.button("Select Tools").clicked() {
                snarl.insert_node(pos, WorkNode::Tools(Default::default()));
                ui.close();
            }
            if ui.button("Invoke Tools").clicked() {
                snarl.insert_node(pos, WorkNode::InvokeTool(Default::default()));
                ui.close();
            }
        });

        ui.menu_button("History", |ui| {
            if ui.button("Create Message").clicked() {
                snarl.insert_node(pos, WorkNode::CreateMessage(Default::default()));
                ui.close();
            }

            if ui.button("Mask History").clicked() {
                snarl.insert_node(pos, WorkNode::MaskChat(Default::default()));
                ui.close();
            }

            if ui.button("Extend History").clicked() {
                snarl.insert_node(pos, WorkNode::ExtendHistory(Default::default()));
                ui.close();
            }

            if ui.button("Side Chat").clicked() {
                snarl.insert_node(pos, WorkNode::GraftChat(Default::default()));
                ui.close();
            }
        });

        ui.menu_button("JSON", |ui| {
            if ui.button("Parse JSON").clicked() {
                snarl.insert_node(pos, WorkNode::ParseJson(Default::default()));
                ui.close();
            }

            if ui.button("Gather JSON").clicked() {
                snarl.insert_node(pos, WorkNode::GatherJson(Default::default()));
                ui.close();
            }

            if ui.button("Validate JSON").clicked() {
                snarl.insert_node(pos, WorkNode::ValidateJson(Default::default()));
                ui.close();
            }

            if ui.button("Transform JSON").clicked() {
                snarl.insert_node(pos, WorkNode::TransformJson(Default::default()));
                ui.close();
            }
        });

        if ui.button("Subgraph").clicked() {
            snarl.insert_node(pos, WorkNode::Subgraph(Default::default()));
            ui.close();
        }

        if ui.button("Preview").clicked() {
            snarl.insert_node(pos, WorkNode::Preview(Default::default()));
            ui.close();
        }

        if ui.button("Output").clicked() {
            snarl.insert_node(pos, WorkNode::Output(Default::default()));
            ui.close();
        }

        if ui.button("Comment").clicked() {
            let node_id = snarl.insert_node(pos, WorkNode::Comment(Default::default()));

            self.shadow = self
                .shadow
                .with_node(&node_id, snarl.get_node_info(node_id));
            ui.close();
        }
    }

    fn has_body(&mut self, node: &WorkNode) -> bool {
        node.as_ui().has_body()
    }

    fn show_body(
        &mut self,
        node: egui_snarl::NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<WorkNode>,
    ) {
        self.edit_ctx.current_node = node;
        self.edit_ctx.disabled = self.shadow.is_disabled(node);

        let enabled = self.can_edit() || matches!(snarl[node], WorkNode::Subgraph(_));

        ui.add_enabled_ui(enabled, |ui| {
            snarl[node].as_ui_mut().show_body(ui, &self.edit_ctx);
            self.shadow = self.shadow.with_node(&node, snarl.get_node_info(node));
        });
    }

    fn has_footer(&mut self, node: &WorkNode) -> bool {
        node.as_ui().has_footer()
    }

    fn show_footer(
        &mut self,
        node: NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<WorkNode>,
    ) {
        self.edit_ctx.current_node = node;
        self.edit_ctx.disabled = self.shadow.is_disabled(node);

        ui.add_enabled_ui(self.can_edit(), |ui| {
            let before = snarl[node].clone();
            snarl[node].as_ui_mut().show_footer(ui, &self.edit_ctx);
            self.shadow = self.shadow.with_node(&node, snarl.get_node_info(node));
            let after = &snarl[node];
            if &before != after {
                tracing::trace!("Change detected in {before:?} to {after:?}");
            }
        });
    }

    fn connect(
        &mut self,
        from: &egui_snarl::OutPin,
        to: &egui_snarl::InPin,
        snarl: &mut Snarl<WorkNode>,
    ) {
        // TODO: cycle check
        if self.can_edit() {
            let remote = &snarl[from.id.node];
            let wire_kind = remote.as_dyn().out_kind(from.id.output);
            let recipient = &snarl[to.id.node];
            if recipient.as_dyn().connect(to.id.input, wire_kind).is_ok() {
                self.drop_inputs(to, snarl);
                snarl.connect(from.id, to.id);
                self.shadow = self.shadow.with_wire(from.id, to.id);
            }
        }
    }

    fn disconnect(
        &mut self,
        from: &egui_snarl::OutPin,
        to: &egui_snarl::InPin,
        snarl: &mut Snarl<WorkNode>,
    ) {
        if self.can_edit() {
            snarl.disconnect(from.id, to.id);
            self.shadow = self.shadow.without_wire(from.id, to.id);
        }
    }

    fn drop_inputs(&mut self, pin: &egui_snarl::InPin, snarl: &mut Snarl<WorkNode>) {
        if self.can_edit() {
            self.shadow = self.shadow.drop_inputs(pin.id);
            snarl.drop_inputs(pin.id);
        }
    }

    fn drop_outputs(&mut self, pin: &egui_snarl::OutPin, snarl: &mut Snarl<WorkNode>) {
        if self.can_edit() {
            self.shadow = self.shadow.drop_outputs(pin.id);
            snarl.drop_outputs(pin.id);
        }
    }

    fn has_node_menu(&mut self, node: &WorkNode) -> bool {
        self.can_edit() && !node.is_protected()
    }

    fn show_node_menu(
        &mut self,
        node: NodeId,
        _inputs: &[egui_snarl::InPin],
        _outputs: &[egui_snarl::OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<WorkNode>,
    ) {
        let targets = self.target_nodes(ui, Some(node));

        let help_link = snarl[node].as_ui().help_link();
        if !help_link.is_empty() {
            ui.add(Hyperlink::from_label_and_url("Help", help_link).open_in_new_tab(true));
        }

        if !matches!(snarl[node], WorkNode::Comment(_)) {
            if self.shadow.is_disabled(node) {
                if ui.button("Enable").clicked() {
                    for node in &targets {
                        self.shadow = self.shadow.enable_node(*node);
                    }
                    ui.close();
                }
            } else if ui.button("Disable").clicked() {
                for node in &targets {
                    if !&snarl[*node].is_protected() {
                        self.shadow = self.shadow.disable_node(*node);
                    }
                }
                ui.close();
            }
        }

        if ui.button("Remove").clicked() {
            self.remove_nodes(ui, snarl, Some(node));
            ui.close();
        }
    }

    fn current_transform(
        &mut self,
        to_global: &mut egui::emath::TSTransform,
        _snarl: &mut Snarl<WorkNode>,
    ) {
        self.transform = *to_global;
    }
}

#[must_use]
pub fn filter_graph(
    graph: ShadowGraph<WorkNode>,
    offset: egui::Vec2,
    keep_nodes: impl AsRef<[NodeId]>,
) -> ShadowGraph<WorkNode> {
    let ShadowGraph { nodes, wires, .. } = graph;
    let keep = keep_nodes.as_ref().iter().collect::<BTreeSet<_>>();
    let nodes: im::OrdMap<NodeId, _> = nodes
        .into_iter()
        .filter(|n| keep.contains(&n.0))
        .map(|(id, meta)| {
            let meta = MetaNode {
                pos: meta.pos - offset,
                ..meta
            };
            (id, meta)
        })
        .collect();
    let wires = wires
        .into_iter()
        .filter(|w| keep.contains(&w.out_pin.node) && keep.contains(&w.in_pin.node))
        .collect();

    let disabled = graph
        .disabled
        .iter()
        .filter(|n| nodes.contains_key(n))
        .cloned()
        .collect();

    ShadowGraph {
        nodes,
        wires,
        disabled,
        ..ShadowGraph::empty()
    }
}

pub fn merge_graphs(
    snarl: &mut Snarl<WorkNode>,
    target: &mut ShadowGraph<WorkNode>,
    offset: egui::Vec2,
    source: ShadowGraph<WorkNode>,
) -> Vec<NodeId> {
    let ShadowGraph { nodes, wires, .. } = source;
    let mut node_map: BTreeMap<NodeId, NodeId> = Default::default();
    let start_id = snarl
        .nodes_ids_data()
        .find(|(_, n)| matches!(n.value, WorkNode::Start(_)))
        .map(|(new_id, _)| new_id);

    for (
        id,
        MetaNode {
            mut value,
            pos,
            open,
        },
    ) in nodes.into_iter()
    {
        // If the start node was part of the selection, preserve connections without duplicating
        if let Some(new_id) = start_id
            && matches!(value, WorkNode::Start(_))
        {
            node_map.insert(id, new_id);
        } else if !value.is_protected() {
            value.as_ui_mut().on_paste();

            let new_id = if open {
                snarl.insert_node(pos + offset, value)
            } else {
                snarl.insert_node_collapsed(pos + offset, value)
            };

            *target = target.with_node(&new_id, snarl.get_node_info(new_id));
            node_map.insert(id, new_id);
        }
    }

    for wire in wires {
        if let Some(from_node) = node_map.get(&wire.out_pin.node)
            && let Some(to_node) = node_map.get(&wire.in_pin.node)
        {
            let src = OutPinId {
                node: *from_node,
                output: wire.out_pin.output,
            };
            let dest = InPinId {
                node: *to_node,
                input: wire.in_pin.input,
            };

            *target = target.with_wire(src, dest);
            snarl.connect(src, dest);
        }
    }

    let disabled = source
        .disabled
        .iter()
        .filter_map(|n| node_map.get(n))
        .cloned()
        .collect();
    let disabled = target.disabled.clone().union(disabled);

    *target = ShadowGraph {
        disabled,
        ..target.clone()
    };

    node_map
        .into_values()
        .filter(|id| start_id != Some(*id))
        .collect()
}
