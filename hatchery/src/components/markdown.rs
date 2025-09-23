use leptos::prelude::*;

#[component]
pub fn Marked(spec: Signal<String>) -> impl IntoView {
    use leptos::html;
    let node_ref: NodeRef<html::Div> = NodeRef::new();

    Effect::new(move |_| {
        if let Some(node) = node_ref.get() {
            let text = markdown::to_html(&spec.get());
            node.set_inner_html(&text);
        }
    });

    view! {
        <div class="prose" node_ref=node_ref />
    }
}
