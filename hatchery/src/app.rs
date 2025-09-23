use leptos::prelude::*;
use leptos_meta::{MetaTags, Script, Stylesheet, Title, provide_meta_context};
use leptos_router::{
    StaticSegment,
    components::{Route, Router, Routes},
};

use crate::pages::HomePage;

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone()/>
                <HydrationScripts options=options islands=true/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    // Provides context that manages stylesheets, titles, meta tags, etc.
    provide_meta_context();
    // Provide websocket connection
    leptos_ws::provide_websocket();

    view! {
        // injects a stylesheet into the document <head>
        // id=leptos means cargo-leptos will hot-reload this stylesheet
        <Stylesheet id="leptos" href="/pkg/hatchery.css"/>

        // sets the document title
        <Title text="Welcome to Leptos"/>

        // Causes hydration mismatch if allowed to auto-render
        <Script src="https://cdn.jsdelivr.net/npm/mermaid/dist/mermaid.min.js"/>
        <Script>"mermaid.initialize({ startOnLoad: false });"</Script>

        // content for this welcome page
        <Router>
            <div class="flex flex-col min-h-screen">
                <nav class="flex-none sticky top-0 z-10">
                    <div class="navbar bg-base-300">
                        <button class="btn btn-ghost text-xl">Untitled</button>
                    </div>
                </nav>
                <div class="w-full">
                    <main class="my-0 mx-auto max-w-5xl border border-amber-200">
                        <Routes fallback=|| "Page not found.".into_view()>
                            <Route path=StaticSegment("") view=HomePage/>
                        </Routes>
                    </main>
                </div>
            </div>
        </Router>
    }
}
