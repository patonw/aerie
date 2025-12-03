use egui::{CentralPanel, Color32};
use egui_graphs::{
    Graph, GraphView, LayoutHierarchical, LayoutStateHierarchical, SettingsInteraction,
    SettingsNavigation,
};
use egui_phosphor::regular::ARROW_CLOCKWISE;
use itertools::Itertools;
use node::TextBlockNode;
use petgraph::{
    Directed, graph::NodeIndex, prelude::StableGraph, stable_graph::DefaultIx,
    visit::IntoNodeReferences,
};
use uuid::Uuid;

use crate::{ChatEntry, ChatHistory};

// TODO: show tags for branch heads

// Account for precision loss when transforming between screen and canvas sizes
const RELAYOUT_THRESHOLD: f32 = 32.0;

pub type Payload = ChatEntry;

pub type GraphViewT<'a> = GraphView<
    'a,
    Payload,
    (),
    petgraph::Directed,
    petgraph::stable_graph::DefaultIx,
    node::TextBlockNode,
    egui_graphs::DefaultEdgeShape,
    LayoutStateHierarchical,
    LayoutHierarchical,
>;

pub struct MessageGraph {
    g: Graph<Payload, (), Directed, DefaultIx, TextBlockNode>,
    idx_map: im::OrdMap<Uuid, NodeIndex>,
    reset: bool,
}

impl MessageGraph {
    pub fn new() -> Self {
        let g = StableGraph::new();

        Self {
            g: Graph::from(&g),
            idx_map: Default::default(),
            reset: true,
        }
    }

    fn refresh_layout(&mut self, ui: &mut egui::Ui) {
        use node::RectangularNode as _;

        // iterate through nodes to get max size, then set layout
        let Some(max_size) = self
            .g
            .g()
            .node_references()
            .map(|(_, node)| node.display().size())
            .reduce(|a, b| egui::vec2(a.x.max(b.x), a.y.max(b.y)))
        else {
            return;
        };

        let row_dist = max_size.y + 32.0;
        let col_dist = max_size.x + 64.0;

        let layout = egui_graphs::get_layout_state::<LayoutStateHierarchical>(ui, None);

        self.reset |= (layout.row_dist - row_dist).abs() > RELAYOUT_THRESHOLD
            || (layout.col_dist - col_dist).abs() > RELAYOUT_THRESHOLD;

        if self.reset {
            self.reset = false;

            egui_graphs::set_layout_state(
                ui,
                LayoutStateHierarchical {
                    triggered: false,
                    row_dist,
                    col_dist,
                    ..layout
                },
                None,
            );
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui) {
        egui::TopBottomPanel::bottom("Controls").show_inside(ui, |ui| {
            if ui
                .button(ARROW_CLOCKWISE)
                .on_hover_text("Reset layout")
                .clicked()
            {
                self.reset = true;
            }
        });

        CentralPanel::default().show_inside(ui, |ui| {
            self.refresh_layout(ui);

            let widget = &mut GraphViewT::new(&mut self.g)
                .with_navigations(
                    &SettingsNavigation::default()
                        .with_fit_to_screen_enabled(false)
                        .with_zoom_and_pan_enabled(true),
                )
                .with_interactions(
                    &SettingsInteraction::default()
                        .with_dragging_enabled(true)
                        .with_node_selection_enabled(true)
                        .with_node_clicking_enabled(true),
                );
            ui.add(widget);
        });
    }

    pub fn update(&mut self, history: &ChatHistory) {
        let colors: im::OrdMap<String, Color32> = node::PALETTE.with(|colors| {
            history
                .branches
                .keys()
                .enumerate()
                .map(|(i, branch)| (branch.to_owned(), colors[i % colors.len()]))
                .collect()
        });

        // Simple case first: only additions. no removals
        for (uuid, entry) in history.store.iter() {
            if let Some(ix) = self.idx_map.get(uuid) {
                if let Some(node) = self.g.node_mut(*ix) {
                    // Only support updating branches currently
                    node.payload_mut().branch = entry.branch.clone();
                    node.set_color(colors.get(&entry.branch).cloned().unwrap_or(Color32::BLACK));
                }
            } else {
                let ix = self.g.add_node(entry.clone());
                self.g
                    .node_mut(ix)
                    .unwrap()
                    .set_color(colors.get(&entry.branch).cloned().unwrap_or(Color32::BLACK));

                self.idx_map.insert(*uuid, ix);
            }
        }

        let removed = self
            .g
            .nodes_iter()
            .filter_map(|(ix, node)| {
                let uuid = node.payload().id;
                if history.store.contains_key(&uuid) {
                    None
                } else {
                    Some((ix, uuid))
                }
            })
            .collect_vec();

        for (ix, uuid) in removed {
            self.idx_map.remove(&uuid);
            self.g.remove_node(ix);
        }

        for (uuid, entry) in history.store.iter() {
            let Some(ix) = self.idx_map.get(uuid) else {
                continue;
            };
            let Some(parent) = &entry.parent else {
                continue;
            };
            let Some(pix) = self.idx_map.get(parent) else {
                continue;
            };

            if !self.g.g().contains_edge(*pix, *ix) {
                self.g.add_edge(*pix, *ix, ());
            }

            // TODO: tag the edge so we can render it differently
            let Some(aside) = &entry.aside else {
                continue;
            };

            let Some(pix) = self.idx_map.get(aside) else {
                continue;
            };

            if !self.g.g().contains_edge(*pix, *ix) {
                self.g.add_edge(*pix, *ix, ());
            }
        }
    }
}

impl Default for MessageGraph {
    fn default() -> Self {
        Self::new()
    }
}

mod node {
    use std::cell::LazyCell;

    use egui::{
        Color32, FontFamily, FontId, Pos2, Rect, Shape, Stroke, Vec2, emath::TSTransform,
        epaint::TextShape,
    };
    use egui_graphs::{DisplayNode, NodeProps};
    use itertools::Itertools as _;
    use petgraph::{EdgeType, stable_graph::IndexType};

    use crate::{
        ChatContent,
        utils::{message_party, message_text},
    };

    use super::Payload;

    thread_local! {
        pub static PALETTE: LazyCell<Vec<egui::Color32>> = const {
            LazyCell::new(|| {
                colorous::CATEGORY10
                    .iter()
                    .map(|c| egui::Color32::from_rgb(c.r, c.g, c.b))
                    .collect_vec()
            })
        };
    }

    pub trait RectangularNode {
        fn loc(&self) -> Pos2;
        fn size(&self) -> egui::Vec2;
        fn contents(&mut self, ctx: &egui_graphs::DrawContext) -> egui::Shape;

        fn padding(&self) -> f32 {
            0.0
        }

        fn set_size(&mut self, size: Vec2) {
            let _ = size;
        }

        fn bounds(&self) -> egui::Vec2 {
            self.size() + Vec2::splat(self.padding() * 2.)
        }

        fn color(&self) -> Option<Color32> {
            None
        }

        fn is_inside(&self, pos: Pos2) -> bool {
            let rect = Rect::from_center_size(self.loc(), self.size());

            rect.contains(pos)
        }

        fn closest_boundary_point(&self, direction: Vec2) -> Pos2 {
            let center = self.loc();
            let bounds = self.bounds();

            if (direction.x.abs() * bounds.y) > (direction.y.abs() * bounds.x) {
                // intersects left or right side
                let x = if direction.x > 0.0 {
                    center.x + bounds.x / 2.0
                } else {
                    center.x - bounds.x / 2.0
                };
                let y = center.y + direction.y / direction.x * (x - center.x);
                Pos2::new(x, y)
            } else {
                // intersects top or bottom side
                let y = if direction.y > 0.0 {
                    center.y + bounds.y / 2.0
                } else {
                    center.y - bounds.y / 2.0
                };
                let x = center.x + direction.x / direction.y * (y - center.y);
                Pos2::new(x, y)
            }
        }

        fn shapes(&mut self, ctx: &egui_graphs::DrawContext) -> Vec<egui::Shape> {
            let mut shape_label = self.contents(ctx);

            // Reposition element to be exactly centered at assigned loc
            let rect = shape_label.visual_bounding_rect();
            let loc = ctx.meta.canvas_to_screen_pos(self.loc());
            shape_label.transform(TSTransform::from_translation(loc - rect.center()));

            let rect = shape_label.visual_bounding_rect();
            self.set_size(rect.size() / ctx.meta.zoom);

            let points = rect_to_points(rect.expand(self.padding() * ctx.meta.zoom));
            let color = self.color().unwrap_or(ctx.ctx.style().visuals.text_color());

            // Don't scale stroke, so that it stands out more at higher zoom levels
            let shape_rect =
                Shape::convex_polygon(points, Color32::default(), Stroke::new(1., color));

            vec![shape_rect, shape_label]
        }
    }

    pub fn as_rect_node<T: RectangularNode>(value: &T) -> &dyn RectangularNode {
        value
    }

    #[derive(Clone)]
    pub struct TextBlockNode {
        text: String,
        loc: Pos2,

        size: Vec2,
        padding: f32,
        color: Option<Color32>,
    }

    impl<N: Clone> From<NodeProps<N>> for TextBlockNode {
        fn from(node_props: NodeProps<N>) -> Self {
            Self {
                text: node_props.label.clone(),
                loc: node_props.location(),

                size: Vec2::ZERO,
                padding: 8.0,
                color: None,
            }
        }
    }

    impl RectangularNode for TextBlockNode {
        fn loc(&self) -> Pos2 {
            self.loc
        }

        fn size(&self) -> egui::Vec2 {
            self.size
        }

        fn set_size(&mut self, size: Vec2) {
            self.size = size;
        }

        fn padding(&self) -> f32 {
            self.padding
        }

        fn color(&self) -> Option<Color32> {
            self.color
        }

        fn contents(&mut self, ctx: &egui_graphs::DrawContext) -> egui::Shape {
            let center = ctx.meta.canvas_to_screen_pos(self.loc);
            let color = ctx.ctx.style().visuals.text_color();

            // create label
            let galley = ctx.ctx.fonts_mut(|f| {
                f.layout(
                    self.text.clone(),
                    FontId::new(ctx.meta.canvas_to_screen_size(10.), FontFamily::Monospace),
                    color,
                    ctx.meta.canvas_to_screen_size(200.),
                )
            });

            // create the shape and add it to the layers
            let shape_label = TextShape::new(center, galley, color);

            shape_label.into()
        }
    }

    impl<E: Clone, Ty: EdgeType, Ix: IndexType> DisplayNode<Payload, E, Ty, Ix> for TextBlockNode {
        fn is_inside(&self, pos: Pos2) -> bool {
            RectangularNode::is_inside(self, pos)
        }

        fn closest_boundary_point(&self, dir: Vec2) -> Pos2 {
            as_rect_node(self).closest_boundary_point(dir)
        }

        fn shapes(&mut self, ctx: &egui_graphs::DrawContext) -> Vec<egui::Shape> {
            RectangularNode::shapes(self, ctx)
        }

        fn update(&mut self, state: &NodeProps<Payload>) {
            self.loc = state.location();
            self.color = state.color();

            match &state.payload.content {
                ChatContent::Message(msg) => {
                    let branch = state.payload.branch.as_str();
                    let party = message_party(msg);
                    let mut text = message_text(msg);
                    if text.len() > 256 {
                        text.truncate(256);
                        text.push_str("...");
                    }
                    self.text = format!("branch: {branch}\nparty: {party}\ntext: {text}");
                }
                other => {
                    let mut text = format!("{:?}", other);
                    if text.len() > 256 {
                        text.truncate(256);
                        text.push_str("...");
                    }
                    self.text = text;
                }
            }
        }
    }

    fn rect_to_points(rect: Rect) -> Vec<Pos2> {
        let top_left = rect.min;
        let bottom_right = rect.max;
        let top_right = Pos2::new(bottom_right.x, top_left.y);
        let bottom_left = Pos2::new(top_left.x, bottom_right.y);

        vec![top_left, top_right, bottom_right, bottom_left]
    }
}

impl super::AppState {
    pub fn message_graph(&mut self, ui: &mut egui::Ui) {
        let app = &mut self.message_graph;

        self.session.view(|history| app.update(history));
        app.render(ui);
    }
}
