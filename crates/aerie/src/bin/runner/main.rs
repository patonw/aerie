use clap::Parser;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _};

mod cli;
mod executor;
#[cfg(feature = "runner-http")]
mod http_server;
#[cfg(feature = "runner-mcp")]
mod mcp_server;
mod output;
mod scoping;

use executor::{check_workflows, execute_workflow};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(EnvFilter::from_default_env())
        .init();

    let args = cli::Args::parse();

    if let Some(env_handle) = &args.env {
        let _ = if env_handle.to_str() == Some("-") {
            dotenvy::from_read(std::io::stdin())
        } else {
            dotenvy::from_path(env_handle)
        };
    }

    match &args.command {
        cli::Command::Check { pretty } => check_workflows(&args, *pretty)?,
        cli::Command::Exec(exec_args) => execute_workflow(&args, exec_args)?,

        #[cfg(feature = "runner-http")]
        cli::Command::Serve(server_args) => http_server::start_server(&args, server_args)?,

        #[cfg(feature = "runner-mcp")]
        cli::Command::MCP(mcp_args) => mcp_server::start(&args, mcp_args)?,
    }

    Ok(())
}
