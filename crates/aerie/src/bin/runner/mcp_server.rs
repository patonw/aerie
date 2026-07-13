use itertools::Itertools;
use rmcp::{
    RoleServer, ServerHandler,
    model::{ErrorData as McpError, PaginatedRequestParams, *},
    service::RequestContext,
};
use std::{collections::BTreeMap, option::Option, sync::Arc};
use typed_builder::TypedBuilder;

use crate::{
    cli::ExecArgs,
    executor::{ExecOverrides, run_loop},
    scoping::{ExecScope, nameless_scope},
};
use aerie::{
    toolbox::parse_or_prompt_schema,
    workflow::{OutputChannel, store::WorkflowStore},
};

#[derive(TypedBuilder)]
pub struct RunnerService {
    scaffolding: Arc<ExecScope>,

    #[builder(default)]
    autoruns: usize,
}

#[allow(clippy::manual_async_fn)] // Can't get lifetimes to agree when using async-trait
impl ServerHandler for RunnerService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_server_info(Implementation::from_build_env())
            .with_instructions("A service to run agentic workflows.")
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        let scaffolding = self.scaffolding.clone();
        scaffolding.workflow_store.as_ref().map(|store| {
            let desc = store.description(name);
            let schema = store.schema(name);
            let schema = object(parse_or_prompt_schema(&schema));

            Tool::new(name.to_string(), desc.into_owned(), Arc::new(schema))
        })
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> {
        async move {
            let scaffolding = self.scaffolding.clone();
            let tools = if let Some(store) = &scaffolding.workflow_store {
                store
                    .names()
                    .map(|name| {
                        let desc = store.description(&name);
                        let schema = store.schema(&name);
                        let schema = object(parse_or_prompt_schema(&schema));

                        Tool::new(name.into_owned(), desc.into_owned(), Arc::new(schema))
                    })
                    .collect_vec()
            } else {
                vec![]
            };

            Ok(ListToolsResult {
                tools,
                ..Default::default()
            })
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let scaffolding = self.scaffolding.clone();
            let store = scaffolding
                .workflow_store
                .as_ref()
                .ok_or(McpError::internal_error("Workflow store missing", None))?;

            let workflow = store.load(&request.name).map_err(|_| {
                McpError::new(ErrorCode::METHOD_NOT_FOUND, request.name.clone(), None)
            })?;

            let schema = store.schema(&request.name);

            let (prompt, images) = if !schema.is_empty()
                && let Ok(schema) = serde_json::from_str(&schema)
            {
                // Should we bail or just ignore invalid schemas?
                let validator = jsonschema::validator_for(&schema).map_err(|err| {
                    McpError::internal_error(format!("Invalid schema: {err:?}"), None)
                })?;

                let args = if let Some(args) = request.arguments.as_ref() {
                    args
                } else {
                    &Default::default()
                };

                let input =
                    serde_json::to_value(args).expect("Could not convert input map to object");

                validator.validate(&input).map_err(|err| {
                    McpError::invalid_request(format!("Validation error: {err:?}"), None)
                })?;

                (input, Default::default())
            } else {
                let prompt = request
                    .arguments
                    .as_ref()
                    .and_then(|args| args.get("prompt").cloned())
                    .unwrap_or_default();
                let images = request
                    .arguments
                    .as_ref()
                    .and_then(|args| args.get("images").cloned())
                    .and_then(|imgs| serde_json::from_value(imgs).ok())
                    .unwrap_or_default();
                (prompt, images)
            };

            let prompt = serde_json::to_string(&prompt)
                .map_err(|e| McpError::internal_error(format!("{e:?}"), None))?;

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

            // TODO: per request autoruns
            let exec_args = ExecArgs {
                autoruns: self.autoruns,
                ..Default::default()
            };

            let exec_task = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                run_loop(
                    &scaffolding,
                    &exec_args,
                    overrides,
                    prompt,
                    images,
                    workflow,
                )?;

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

                drop(exec_args);
                drop(scaffolding);
                Ok(outputs)
            });

            let result = exec_task.await;

            let outputs: Result<Vec<_>, serde_json::Error> = match result {
                Err(err) => Err(McpError::internal_error(
                    format!("Unable to launch runner: {err:?}"),
                    None,
                ))?,
                Ok(Err(err)) => Err(McpError::internal_error(
                    format!("Error during workflow execution: {err:?}"),
                    None,
                ))?,
                Ok(Ok(outputs)) => outputs
                    .into_iter()
                    .map(|obj| serde_json::to_value(&obj))
                    .collect(),
            };

            let content = outputs
                .map_err(|err| McpError::internal_error(format!("{err:?}"), None))?
                .into_iter()
                .map(Content::json)
                .collect::<Result<Vec<_>, McpError>>()?;

            Ok(CallToolResult::success(content))
        })
    }
}

pub fn start(args: &crate::cli::Args, mcp_args: &crate::cli::McpServerArgs) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let scaffolding = nameless_scope(args, rt.handle())?;

    let shutdown = scaffolding.shutdown.clone();
    rt.spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("Received interrupt. Beginning shutdown sequence...");
        shutdown.cancel();
        tracing::info!("Shutdown flag set 2");
        // tracing::info!("Stopping server");
        // token.cancel();
    });

    let result = rt.block_on(async move {
        use rmcp::{ServiceExt, transport::stdio};
        let shutdown = scaffolding.shutdown.clone();
        let runner_service = RunnerService::builder()
            .scaffolding(scaffolding.clone())
            .autoruns(mcp_args.autoruns)
            .build();

        let service = runner_service.serve_with_ct(stdio(), shutdown).await?;
        service.waiting().await?;

        Ok(())
    });

    tracing::info!("Server stopped");
    result
}
