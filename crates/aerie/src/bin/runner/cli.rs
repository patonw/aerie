use std::option::Option;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

#[skip_serializing_none]
#[derive(clap::Args, Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecArgs {
    /// The workflow file to run
    pub workflow: String,

    /// Initial user prompt if required by the workflow
    #[serde(default)]
    #[arg(short, long, visible_alias("prompt"))]
    pub input: Option<String>,

    /// Path to file containing the initial prompt
    #[serde(default)]
    #[arg(short = 'I', long)]
    pub input_file: Option<PathBuf>,

    /// Either a file path or data url
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[arg(long, visible_alias("image"), action = clap::ArgAction::Append)]
    pub images: Vec<String>,

    /// Save outputs as individual files in a directory
    #[serde(default)]
    #[arg(short, long)]
    pub out_dir: Option<PathBuf>,

    /// Number of extra turns to run chained workflows
    #[serde(default, skip_serializing_if = "is_zero")]
    #[arg(short, long, default_value_t = 0)]
    pub autoruns: usize,

    /// Prints an additional object containing the next workflow after the last run
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[arg(short = 'n', long, action = clap::ArgAction::SetTrue, default_value_t = false)]
    pub show_next: bool,

    /// Include workflow ids in output
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[arg(long, action = clap::ArgAction::SetTrue, default_value_t = false)]
    pub show_ids: bool,

    /// Pretty print console output
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[arg(short, long, action = clap::ArgAction::SetTrue, default_value_t = false)]
    pub pretty: bool,
}

#[inline]
pub(crate) fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[derive(clap::Args, Clone, Debug)]
pub struct HttpServerArgs {
    #[arg(short, long, default_value_t = String::from("localhost"))]
    pub(crate) host: String,

    #[arg(short, long, default_value_t = 8058)]
    pub(crate) port: u32,
}

#[derive(clap::Args, Clone, Debug)]
pub struct McpServerArgs {
    /// Listen with streaming HTTP transport instead of STDIO
    #[arg(long, action = clap::ArgAction::SetTrue, default_value_t = false)]
    pub(crate) http: bool,

    #[arg(short, long, default_value_t = 3)]
    pub(crate) autoruns: usize,
}

#[non_exhaustive]
#[derive(Subcommand, Clone, Debug)]
pub enum Command {
    /// Loads tool providers and checks workflows in store
    Check {
        #[arg(short, long, action = clap::ArgAction::SetTrue, default_value_t = false)]
        pretty: bool,
    },

    /// Execute a workflow and dump results to console or a directory
    Exec(ExecArgs),

    #[cfg(feature = "runner-http")]
    #[command(visible_alias = "http")]
    Serve(HttpServerArgs),

    #[allow(clippy::upper_case_acronyms)]
    #[cfg(feature = "runner-mcp")]
    MCP(McpServerArgs),
}

/// A minimalist workflow runner that dumps outputs to the console as a JSON object.
///
/// If you need post-processing, use external tools like jq, sed and awk.
#[derive(Parser, Debug)]
#[command(version, about)]
pub(crate) struct Args {
    /// Configuration file containing tool providers and default agent settings
    #[arg(short, long)]
    pub(crate) config: Option<PathBuf>,

    /// Configuration file containing tool providers and default agent settings
    #[arg(short, long)]
    pub(crate) profile: Option<String>,

    /// An ephemeral file handle to dotenv formatted secrets
    #[arg(long, short)]
    pub(crate) env: Option<PathBuf>,

    /// Directory containing workflows
    #[arg(short, long)]
    pub(crate) workflows: Option<PathBuf>,

    /// Directory containing tool provider definitions
    #[arg(short, long)]
    pub(crate) tools: Option<PathBuf>,

    /// A session to use in the workflow.
    /// Updates are discarded unless `--update` is also used.
    #[arg(short, long)]
    pub(crate) session: Option<PathBuf>,

    /// The session branch to use
    #[arg(short, long)]
    pub(crate) branch: Option<String>,

    /// Save updates to the session after running the workflow.
    #[arg(long, action)]
    pub(crate) update_session: bool,

    /// Model(s) to use in the workflow.
    ///
    /// Entries can by tagged with roles by appending `=role1,role2,etc.`
    /// An untagged entry is interpreted as the default model.
    ///
    /// Examples:
    ///   -m openrouter/openrouter/free
    ///   -m openrouter/openrouter/free=default,creative
    #[arg(short, long, visible_alias("model"), verbatim_doc_comment)]
    pub(crate) models: Vec<String>,

    /// Default language model temperature
    #[arg(short = 'T', long)]
    pub(crate) temperature: Option<f64>,

    #[command(subcommand)]
    pub command: Command,
}
