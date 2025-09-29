use leptos::task::spawn_local;
use leptos::{html, logging, prelude::*};
use serde::{Deserialize, Serialize};
use textwrap::dedent;

use crate::components::{Marked, Mermaid};

const EXAMPLE_TEXT: &str = "\
Try to put a blank line before...

> This is a blockquote

...and after a blockquote.
1. First item
2. Second item
3. Third item
    - Indented item
    - Indented item
4. Fourth item
";

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct HistoryEntry {
    name: String,
    number: u16,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct History {
    entries: Vec<HistoryEntry>,
    stop: bool,
}

// // This is just becoming more trouble than its worth ...back to egui
// #[derive(Clone, Serialize, Deserialize, Debug, Default)]
// pub struct ChatSession {
//     history: Vec<Message>,
// }

#[component]
pub fn HomePage() -> impl IntoView {
    let interrupt = leptos_ws::BiDirectionalSignal::new("interrupt", false).unwrap();

    let task_count = leptos_ws::BiDirectionalSignal::new("task_count", 0 as u32).unwrap();

    let scroll = RwSignal::new(true);
    let count = leptos_ws::BiDirectionalSignal::new("count", 0 as i32).unwrap();

    let history = leptos_ws::BiDirectionalSignal::new("history", History::default()).unwrap();

    // let chat = leptos_ws::BiDirectionalSignal::new("chat", ChatSession::default()).unwrap();

    let last_message: NodeRef<html::Div> = NodeRef::new();

    {
        let count = count.clone();
        Effect::new(move |_| {
            if count.get() > 10 {
                logging::log!("Resetting value");
                count.update(move |value| *value = 0);
            }
        });
    }

    Effect::new(move |_| {
        if let Some(target) = last_message.get()
            && scroll.get()
        {
            // footer masks content when using bottom alignment
            target.scroll_into_view_with_bool(true);
        }
    });

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

    let sig_int = move |_| {
        interrupt.update(|v| *v = true);
    };

    let start_counter = move |_| {
        spawn_local(async move {
            // let response = shout().await.unwrap();
            // logging::log!("LLM client: {response}");
            update_count().await.unwrap();
        });
    };

    let start_history = {
        let history = history.clone();
        move |_| {
            history.update(|hist| hist.stop = false);
            spawn_local(async move {
                let _ = update_history().await.unwrap();
            });
        }
    };
    let stop_history = {
        let history = history.clone();
        move |_| {
            history.update(|hist| hist.stop = true);
        }
    };

    let count = move || count.get();
    view! {
        <div class="w-full flex flex-col pb-24">
            <div class="card w-96 bg-primary text-primary-content card-md shadow-md">
                <div class="card-body">
                    <h1 class="card-title">"Count: " {count}</h1>
                    <p>A card component has a figure, a body part, and inside body there are title and actions parts</p>
                    <div class="justify-end card-actions">
                    <button class="btn" on:click=start_counter>Start Counter</button>
                    <button class="btn" on:click=sig_int>Stop</button>
                    </div>
                </div>
            </div>

            <div class="chat chat-end">
                <div class="chat-bubble">
                    <Mermaid spec=Signal::derive(spec) />
                    <Marked spec=Signal::stored(format!("{EXAMPLE_TEXT}"))/>
                </div>
            </div>
            <For each=move || history.get().entries key=|it| it.number children=move |it| { view! {
                <div class="chat chat-start">
                    <div class="chat-bubble" node_ref=last_message>
                        { format!("{it:?}") }
                    </div>
                </div>
            }} />


        </div>

        <footer class="fixed bottom-0 mx-auto max-w-5xl w-full h-24 p-4border border-slate-300">
        <nav class="flex justify-between items-center bg-base-200 text-base-content w-full h-full">
            <button class="btn btn-secondary btn-xs" on:click=start_history>Start History Changes</button>
            <Show when=move || { task_count.get() > 0 }><span class="loading loading-dots loading-sm"></span></Show>
            <button class="btn" on:click=stop_history>Stop History</button>
            <label class="label">
                <input type="checkbox" bind:checked=scroll class="toggle toggle-primary" />
                Scroll
            </label>
        </nav>
        </footer>
    }
}

#[server]
async fn update_count() -> Result<(), ServerFnError> {
    use std::time::Duration;
    use tokio::time::sleep;
    let count = leptos_ws::BiDirectionalSignal::new("count", 0 as i32).unwrap();
    let interrupt = leptos_ws::BiDirectionalSignal::new("interrupt", false).unwrap();
    interrupt.update(|v| *v = false);
    let task_count = leptos_ws::BiDirectionalSignal::new("task_count", 0 as u32).unwrap();
    task_count.update(|v| *v += 1);
    for _ in 0..20 {
        if interrupt.get() {
            break;
        }
        count.update(move |value| *value += 1);
        sleep(Duration::from_secs(1)).await;
    }
    task_count.update(|v| *v -= 1);
    Ok(())
}

#[server]
async fn update_history() -> Result<(), ServerFnError> {
    use std::time::Duration;
    use tokio::time::sleep;
    let history = leptos_ws::BiDirectionalSignal::new("history", History::default()).unwrap();
    let task_count = leptos_ws::BiDirectionalSignal::new("task_count", 0 as u32).unwrap();
    task_count.update(|v| *v += 1);
    for i in 0..64 {
        if history.get().stop {
            break;
        }
        history.update(move |value| {
            let last_id = value.entries.last().map(|it| it.number).unwrap_or_default();
            value.entries.push(HistoryEntry {
                name: format!("{}", i * 2).to_string(),
                number: last_id + 1 as u16,
            })
        });
        sleep(Duration::from_millis(1000)).await;
    }
    task_count.update(|v| *v -= 1);
    Ok(())
}

// #[server]
// pub async fn shout() -> Result<String, ServerFnError> {
//     use rig::agent::Agent;
//     use rig::client::completion::CompletionModelHandle;
//     use rig::completion::Prompt;
//
//     let agent: Option<Agent<CompletionModelHandle<'_>>> = use_context();
//     let model = agent.unwrap();
//
//     // let chat = leptos_ws::BiDirectionalSignal::new("chat", ChatSession::default()).unwrap();
//     let history = leptos_ws::BiDirectionalSignal::new("history", History::default()).unwrap();
//     let task_count = leptos_ws::BiDirectionalSignal::new("task_count", 0 as u32).unwrap();
//     task_count.update(|v| *v += 1);
//
//     // let client = Arc::new(ollama::Client::from_env());
//     // let model = client.agent("devstral:latest").build();
//
//     // Prompt the model and print its response
//     let response = model
//         .prompt("Who are you?")
//         .await
//         .expect("Failed to prompt LLM");
//
//     logging::log!("LLM: {response}");
//
//     let text = response.clone();
//     history.update(move |value| {
//         let last_id = value.entries.last().map(|it| it.number).unwrap_or_default();
//         value.entries.push(HistoryEntry {
//             name: text.clone(),
//             number: last_id + 1 as u16,
//         })
//     });
//
//     chat.update(move |state| state.history.push(Message::assistant(text.clone())));
//
//     task_count.update(|v| *v -= 1);
//     Ok(response)
// }
