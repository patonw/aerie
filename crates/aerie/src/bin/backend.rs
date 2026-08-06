use clap::Parser as _;
use std::{
    net::SocketAddr,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use tracing::instrument;

use aerie::{
    client::{
        ClientAction, ClientUpdate, EditorPatch, StateDump, StatePatch, SwitchData, UpdateData,
    },
    config::Args,
    storage::CachedDirStore as _,
    ui::state::WorkflowState,
};
use axum::{
    extract::{
        ConnectInfo, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
};
use axum_extra::{TypedHeader, headers};
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

#[derive(Debug, Clone)]
pub struct SharedState {
    pub rt: tokio::runtime::Handle,
    pub args: Args,
    pub settings_path: PathBuf,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();

    let settings_path = args.config.clone().unwrap_or(
        dirs::config_dir()
            .map(|p| p.join("aerie"))
            .unwrap_or_default()
            .join("settings.toml"),
    );

    // Shhh...
    let _ = dotenvy::from_path(settings_path.with_file_name(".env"));

    if let Some(env_handle) = &args.env {
        let _ = if env_handle.to_str() == Some("-") {
            dotenvy::from_read(std::io::stdin())
        } else {
            dotenvy::from_path(env_handle)
        };
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let shutdown: CancellationToken = Default::default();
    let shutdown_ = shutdown.clone();

    rt.spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("Received interrupt. Beginning shutdown sequence...");
        shutdown_.cancel();
    });

    let shared = SharedState {
        rt: rt.handle().clone(),
        args,
        settings_path,
    };

    // build our application with some routes
    use axum::routing::*;
    let router = Router::new()
        // .fallback_service(ServeDir::new(assets_dir).append_index_html_on_directories(true))
        .route("/ws", any(ws_handler))
        // logging so we can see what's going on
        // .layer(
        //     TraceLayer::new_for_http()
        //         .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        // )
        .with_state(Arc::new(shared));

    rt.block_on(async {
        let _guard = rt.enter();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
            .await
            .unwrap();
        tracing::debug!("listening on {}", listener.local_addr().unwrap());
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = shutdown.cancelled().await;
        })
        .await
    })?;

    Ok(())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(shared): State<Arc<SharedState>>,
) -> impl IntoResponse {
    let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
        user_agent.to_string()
    } else {
        String::from("Unknown browser")
    };
    tracing::info!("`{user_agent}` at {addr} connected.");
    // finalize the upgrade process by returning upgrade callback.
    // we can customize the callback by sending additional info such as address.

    let factory = aerie::app::AppFactory::builder()
        .name("aerie")
        .args(shared.args.clone())
        .rt(shared.rt.clone())
        .settings_path(shared.settings_path.clone())
        .min_size(egui::vec2(800.0, 400.0))
        .logging_init(Rc::new(|_| {}))
        .build();
    let app = factory.make_app().unwrap();

    ws.on_upgrade(move |socket| handle_socket(app, socket, addr))
}

// TODO: switch to binary serialization. Will require forking data types:
// Must not use enum tags. Must not omit default fields.
async fn handle_socket(mut app: aerie::app::App, mut socket: WebSocket, _who: SocketAddr) {
    let dump_result = send_dump(&app, &mut socket).await;
    // let _ = send_switch(&app.app_state.workflows, &mut socket).await;

    let mut waiting = true;
    let mut interval = tokio::time::interval(Duration::from_millis(1000));
    let task_count = app.app_state.task_count.clone();

    tracing::debug!("Sending initial dump {dump_result:?}",);

    // let bytes = postcard::to_allocvec(&update).unwrap();
    // tracing::debug!(
    //     "Sending initial dump {:?}",
    //     socket.send(Message::Binary(bytes.into())).await
    // );

    loop {
        tokio::select! {
            // TODO: use broadcast get updates for chat scratch
            _ = interval.tick() => {
                if waiting  && task_count.load(Ordering::Relaxed) == 0 {
                    tracing::info!("Finished waiting message");
                    waiting = false;
                    let _ = patch_all(&app, &mut socket).await;
                }
            }
            res = socket.recv() => {
                match res {
                    Some(msg) => {
                        tracing::info!(?msg, "Received message");
                        match msg {
                            Ok(Message::Binary(bytes))
                                if let Ok(payload) = postcard::from_bytes::<ClientAction>(&bytes) =>
                            {
                                tracing::info!("Got client action {payload:?}");
                            }
                            Ok(Message::Text(txt))
                                if let Ok(payload) = serde_json::from_str::<ClientAction>(txt.as_str()) =>
                            {
                                use aerie::client::ActionData::*;
                                tracing::info!("Got json action {payload:?}");
                                let mut need_patch = false;

                                match payload.action {
                                    Dispatch(app_events) => {
                                        for event in app_events {
                                            use aerie::events::AppEvent::*;
                                            match event {
                                                SwitchWorkflow(name) => {
                                                    app.app_state.workflows.switch(&name);

                                                    let _ = send_switch(&app.app_state.workflows, &mut socket).await;
                                                },
                                                ev @ (EnterSubgraph(_) | LeaveSubgraph(_) | NodesChanged(_, _) | GraphSnapshot(_) | WorkflowSnapshot(_)) => {
                                                    app.app_state.events.insert(ev);
                                                }
                                                ev => {
                                                    app.app_state.events.insert(ev);
                                                    need_patch = true;
                                                }
                                            }
                                        }

                                        app.app_state.handle_events();

                                        if need_patch {
                                            waiting = task_count.load(Ordering::Relaxed) > 0;
                                            let _ = patch_all(&app, &mut socket).await;
                                        }
                                    }
                                }
                            }
                            Ok(Message::Close(_)) => {}
                            unk => {
                                tracing::info!("Not sure what to do with {unk:?}");
                            }
                        }
                    }
                    None => { break;}
                }
            }
        }
    }
    // while let Some(msg) = socket.recv().await {}
}

async fn send_switch<W: aerie::workflow::store::WorkflowStore>(
    work_state: &WorkflowState<W>,
    socket: &mut WebSocket,
) -> Result<(), axum::Error> {
    let payload = SwitchData::builder()
        .name(work_state.editing.clone())
        .baseline(work_state.baseline.clone())
        .build();

    let update = ClientUpdate {
        update: UpdateData::Switch(payload),
    };

    let _ = socket
        .send(Message::Text(
            serde_json::to_string(&update).unwrap().into(),
        ))
        .await;
    Ok(())
}

async fn send_dump(app: &aerie::app::App, socket: &mut WebSocket) -> Result<(), axum::Error> {
    let tool_store = app.app_state.tools.clone();

    let flow_store = &app.app_state.workflows.store;
    let workflows = flow_store
        .names()
        .filter_map(|n| {
            flow_store
                .load(n.as_ref())
                .ok()
                .map(|w| (n.to_string(), w.metadata.as_ref().clone()))
        })
        .collect();

    let state = StateDump::builder()
        .app_data(app.app_state.data.clone())
        .tools(tool_store)
        .workflows(workflows)
        .prompt(app.app_state.prompt.clone())
        .task_count(app.app_state.task_count.load(Ordering::Relaxed))
        .build();
    let update = ClientUpdate {
        update: UpdateData::Dump(state),
    };

    socket
        .send(Message::Text(
            serde_json::to_string(&update).unwrap().into(),
        ))
        .await
}

#[instrument(skip_all)]
async fn patch_all(app: &aerie::app::App, socket: &mut WebSocket) -> Result<(), axum::Error> {
    let tool_store = app.app_state.tools.clone();

    let flow_store = &app.app_state.workflows.store;
    let workflows = flow_store
        .names()
        .filter_map(|n| {
            flow_store
                .load(n.as_ref())
                .ok()
                .map(|w| (n.to_string(), w.metadata.as_ref().clone()))
        })
        .collect();

    // TODO: Only set this on change
    let baseline = app.app_state.workflows.baseline.clone();
    let shadow = if app.app_state.workflows.has_changes() {
        Some(app.app_state.workflows.shadow.clone())
    } else {
        None
    };

    tracing::info!("Initial workflow name {}", app.app_state.workflows.editing);

    let editor = EditorPatch::builder()
        .name(app.app_state.workflows.editing.clone())
        .shadow(shadow)
        .baseline(baseline)
        .build();

    // tracing::info!("Sending editor patch {editor:?}");
    // {
    //     let txt = serde_json::to_string_pretty(&editor).unwrap();
    //     tracing::info!("Editor patch txt: {txt}");
    //     let patch: EditorPatch = serde_json::from_str(&txt).unwrap();
    //
    //     tracing::info!("round trip editor patch {patch:?}");
    // }

    let state = StatePatch::builder()
        .app_data(app.app_state.data.clone())
        .tools(tool_store)
        .workflows(workflows)
        .prompt(app.app_state.prompt.clone())
        .task_count(app.app_state.task_count.load(Ordering::Relaxed))
        .config_state(app.app_state.state.clone())
        .editor(editor)
        .build();

    let update = ClientUpdate {
        update: UpdateData::Patch(state),
    };

    socket
        .send(Message::Text(
            serde_json::to_string(&update).unwrap().into(),
        ))
        .await
}
