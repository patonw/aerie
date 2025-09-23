use leptos::task::spawn_local;
use leptos::{html, logging, prelude::*};
use leptos_meta::{provide_meta_context, MetaTags, Script, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes},
    StaticSegment,
};
use serde::{Deserialize, Serialize};
use textwrap::dedent;

use crate::components::mermaid::Mermaid;
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct HistoryEntry {
    name: String,
    number: u16,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct History {
    entries: Vec<HistoryEntry>,
}

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
        <Stylesheet id="leptos" href="/pkg/nursery.css"/>

        // sets the document title
        <Title text="Welcome to Leptos"/>

        // Causes hydration mismatch if allowed to auto-render
        <Script src="https://cdn.jsdelivr.net/npm/mermaid/dist/mermaid.min.js"/>
        <Script>"mermaid.initialize({ startOnLoad: false });"</Script>

        // content for this welcome page
        <Router>
            <main>
                <Routes fallback=|| "Page not found.".into_view()>
                    <Route path=StaticSegment("") view=HomePage/>
                </Routes>
            </main>
        </Router>
    }
}

#[component]
fn HomePage() -> impl IntoView {
    logging::log!("Is this working?");
    let count = leptos_ws::BiDirectionalSignal::new("count", 0 as i32).unwrap();

    let history =
        leptos_ws::BiDirectionalSignal::new("history", History { entries: vec![] }).unwrap();

    {
        let count = count.clone();
        Effect::new(move |_| {
            logging::log!("Value {}", count.get());
            if count.get() > 10 {
                count.update(move |value| *value = 0);
                logging::log!("Resetting");
            }
        });
    }

    let spec = {
        let count = count.clone();
        move || {
            dedent(&format!(
                "sequenceDiagram
                    Alice->>John: Hello John, how are you?
                    John-->>Alice: Great!
                    Alice-)John: See you later! {}",
                count.get()
            ))
        }
    };

    let count = move || count.get();
    view! {

        <h1>"Welcome to Leptos!"</h1>

        <Mermaid spec=Signal::derive(spec) />

        <button class="btn" on:click=move |_| {
            spawn_local(async move {
                update_count().await.unwrap();
            });
        }>Start Counter</button>
        <h1>"Count: " {count}</h1>
        <button class="btn btn-secondary btn-xs" on:click=move |_| {
            spawn_local(async move {
             let _ = update_history().await.unwrap();
            });
        }>Start History Changes</button>
        <p>{move || format!("history: {:?}",history.get())}</p>

    }
}

#[server]
async fn update_count() -> Result<(), ServerFnError> {
    use std::time::Duration;
    use tokio::time::sleep;
    let count = leptos_ws::BiDirectionalSignal::new("count", 0 as i32).unwrap();
    for _ in 0..20 {
        count.update(move |value| *value += 1);
        sleep(Duration::from_secs(1)).await;
    }
    Ok(())
}

#[server]
async fn update_history() -> Result<(), ServerFnError> {
    use std::time::Duration;
    use tokio::time::sleep;
    let history =
        leptos_ws::BiDirectionalSignal::new("history", History { entries: vec![] }).unwrap();
    for i in 0..255 {
        history.update(move |value| {
            value.entries.push(HistoryEntry {
                name: format!("{}", i * 2).to_string(),
                number: i * 2 + 1 as u16,
            })
        });
        sleep(Duration::from_millis(1000)).await;
    }
    Ok(())
}
