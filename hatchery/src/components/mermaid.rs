use leptos::prelude::*;

use crate::utils::mermaid;

#[component]
pub fn Mermaid(#[prop(into)] spec: Signal<String>) -> impl IntoView {
    use leptos::html::*;
    let node_ref: NodeRef<Pre> = NodeRef::new();
    Effect::new(move |_| {
        // mermaid::contentLoaded();
        let value = spec.get();

        if let Some(node) = node_ref.get() {
            node.set_inner_html(&value);
            let _ = node.remove_attribute("data-processed");

            // TODO: target this node specifically
            mermaid::run();

            // mermaid::render(&JsValue::from_serde(&json!({ "nodes": [node]})).unwrap());
        }
    });

    // TODO: force minimum width for legibility
    pre().node_ref(node_ref).class("mermaid")
}

// Can't figure out how to turn children to nodes
// #[component]
// pub fn Moomaid(id: String, children: Children) -> impl IntoView {
//     use leptos::html::*;
//     let node_ref: NodeRef<html::Pre> = NodeRef::new();
//     logging::log!("Children is {inner}");
//     Effect::new(move |_| {
//         mermaid::run();
//
//         let node = node_ref.get().expect("Pre should be mounted");
//
//         node.replace_children_with_node_1(&children().to_html_stream_in_order_branching);
//     });
//
//     pre().node_ref(node_ref).id(id).class("mermaid")
// }
