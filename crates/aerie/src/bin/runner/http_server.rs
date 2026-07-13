use aerie::storage::CachedDirStore;
use aerie::workflow::OutputChannel;
use aerie::workflow::runner::RunEventCast;
use aerie::workflow::store::WorkflowStore;
use anyhow::Context as _;
use async_stream::stream;
use axum::extract::{Json, State};
use axum::http::HeaderMap;
use axum::response::Sse;
use axum::response::sse::Event;
use axum::{
    Router,
    http::StatusCode,
    response::{self, IntoResponse},
    routing::{get, post},
};
use glob::Pattern;
use itertools::Itertools as _;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::block_in_place;

use crate::{
    cli::{Args, ExecArgs, HttpServerArgs},
    executor::{ExecOverrides, run_loop},
    scoping::{ScopeFactory, UserData, make_scope_factory},
};

struct MaybeUserData(anyhow::Result<Option<UserData>>);

impl From<axum::http::HeaderMap> for MaybeUserData {
    fn from(headers: axum::http::HeaderMap) -> Self {
        // TODO: config or build feature?
        if let Some(user_header) = headers.get("X-User-Name") {
            let env = headers
                .get_all("X-User-Env")
                .iter()
                .filter_map(|kv| kv.to_str().ok())
                .filter_map(|kv| kv.split_once("="))
                .map(|(k, v)| (k.to_string(), v.to_string()));

            let result = user_header
                .to_str()
                .context("Invalid header value")
                .map(|name| name.into())
                .map(|ud: UserData| ud.with_env(env))
                .map(Some);

            return Self(result);
        }

        Self(Ok(None))
    }
}

#[derive(Debug, Deserialize)]
struct ExecutionParams {
    #[serde(default, rename = "#events")]
    events: Vec<String>,

    #[serde(flatten)]
    args: ExecArgs,
}

async fn execute_handler(
    headers: HeaderMap,
    State(scope_factory): State<Arc<ScopeFactory>>,
    Json(payload): Json<ExecArgs>,
) -> impl IntoResponse {
    let Ok(user_data) = MaybeUserData::from(headers).0 else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    let scaffolding = block_in_place(|| scope_factory.user_scope(&user_data.unwrap_or_default()));
    // let scaffolding_ = scaffolding.lock().await;
    let Some(workflow_store) = &scaffolding.workflow_store else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    tracing::info!("Payload is {payload:?}");

    let Ok(workflow) = CachedDirStore::load(workflow_store, &payload.workflow) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let prompt = payload.input.clone().unwrap_or_default();
    let images = payload.images.clone();
    // drop(scaffolding_);

    let (chan_tx, chan_rx) = flume::unbounded();
    let overrides = ExecOverrides::builder()
        .run_ctx_fn({
            Arc::new(move |mut run_ctx| {
                let outputs = OutputChannel::default();
                chan_tx
                    .send(outputs.clone())
                    .expect("Could not send output channel");
                run_ctx.outputs = outputs.clone();
                run_ctx
            })
        })
        .build();

    let scaffolding = scaffolding.clone();
    let shutdown = scaffolding.shutdown.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        run_loop(&scaffolding, &payload, overrides, prompt, images, workflow)
    });

    let result = tokio::select! {
        res = &mut task => {
            res
        }
            _ = shutdown.cancelled() => {
            tracing::info!("Received shutdown signal. Cancelling run...");
            return (StatusCode::SERVICE_UNAVAILABLE, "Server shutting down").into_response();
        }
    };

    if let Err(err) = result {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{err:?}")).into_response();
    }

    if let Ok(Err(err)) = result {
        // TODO: more nuance
        return (StatusCode::BAD_REQUEST, format!("{err:?}")).into_response();
    }

    let outputs = chan_rx
        .drain()
        .map(|outputs| {
            outputs
                .receiver()
                .drain()
                .map(|(k, v)| (k, serde_json::Value::try_from(v).unwrap()))
                .collect::<BTreeMap<_, _>>()
        })
        .collect_vec();

    response::Json(outputs).into_response()
}

async fn sse_handler(
    headers: HeaderMap,
    State(scope_factory): State<Arc<ScopeFactory>>,
    Json(payload): Json<ExecutionParams>,
) -> impl IntoResponse //Sse<impl Stream<Item = Result<Event, Infallible>>>
{
    let Ok(user_data) = MaybeUserData::from(headers).0 else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    let scaffolding = block_in_place(|| scope_factory.user_scope(&user_data.unwrap_or_default()));
    let Some(workflow_store) = &scaffolding.workflow_store else {
        tracing::error!("Server started without workflow directory");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    tracing::info!("Payload is {payload:?}");

    let Ok(workflow) = CachedDirStore::load(workflow_store, &payload.args.workflow) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let prompt = payload.args.input.clone().unwrap_or_default();
    let images = payload.args.images.clone();

    let (run_tx, mut run_rx) = async_broadcast::broadcast(64);
    run_rx.set_overflow(true);

    let (chan_tx, chan_rx) = flume::unbounded();
    let overrides = ExecOverrides::builder()
        .run_ctx_fn({
            Arc::new(move |mut run_ctx| {
                let outputs = OutputChannel::default();
                chan_tx
                    .send(outputs.clone())
                    .expect("Could not send output channel");
                run_ctx.outputs = outputs.clone();
                run_ctx
            })
        })
        .run_events(RunEventCast(run_tx))
        .build();

    let start = Instant::now();
    let scaffolding = scaffolding.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        run_loop(
            &scaffolding,
            &payload.args,
            overrides,
            prompt,
            images,
            workflow,
        )
    });

    let event_filters = payload
        .events
        .iter()
        .filter_map(|s| Pattern::new(s).ok())
        .collect_vec();

    let resp_stream = stream! {
        let mut outputs = OutputChannel::default();
        let mut out_rx = outputs.receiver();

        loop {
            tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                let elapsed = start.elapsed().as_secs();
                yield Ok(Event::default().comment(format!("keep-alive @ {elapsed}s")));
            }
                Ok(out_chan) = chan_rx.recv_async() => {
                    outputs = out_chan;
                    out_rx = outputs.receiver();
                    yield Ok(Event::default().comment("Switching to new output channel"));
                }
                Ok((k,v)) = out_rx .recv_async() => {
                    yield Event::default().event("output").json_data(json!({k: v}));
                }
                Ok(event) = run_rx.recv() => {
                    if event.filter(&event_filters) {
                        yield Event::default().event("run-event").json_data(
                            event.with_elapsed(start.elapsed()));
                    }
                }
                res = &mut task => {
                    let elapsed = start.elapsed().as_secs_f32();
                    let msg = format!("Finished task in {elapsed:.2}s with result: {res:?}");
                    tracing::info!("{msg}");
                    yield Ok::<_, axum::Error>(Event::default().comment(msg.escape_default().to_string()));
                    break;
                }
            }
        }
    };

    Sse::new(resp_stream).into_response()
}

pub fn start_server(args: &Args, server_args: &HttpServerArgs) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let scope_factory = Arc::new(make_scope_factory(args, rt.handle())?);

    // TODO: require workflow store

    let shutdown = scope_factory.shutdown.clone();
    rt.spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("Received interrupt. Beginning shutdown sequence...");
        shutdown.cancel();
    });

    let shutdown = scope_factory.shutdown.clone();

    rt.block_on(async move {
        let app = Router::new()
            .route(
                "/list",
                get({
                    let scope_factory = scope_factory.clone();
                    || async move {
                        let scope = scope_factory.no_tools();
                        // let scaffolding_ = scaffolding.lock().await;
                        let Some(workflow_store) = &scope.workflow_store else {
                            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                        };

                        response::Json(
                            WorkflowStore::names(workflow_store)
                                .map(|s| s.to_string())
                                .collect_vec(),
                        )
                        .into_response()
                    }
                }),
            )
            .route("/execute", post(execute_handler))
            .route("/sse", post(sse_handler))
            .with_state(scope_factory.clone());

        let HttpServerArgs { host, port } = server_args;
        let addr = format!("{host}:{port}");
        let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
        println!("Listening on {addr}");

        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown.cancelled().await;
            })
            .await
            .unwrap();

        tracing::info!("Server stopped.");

        Ok(())
    })
}
