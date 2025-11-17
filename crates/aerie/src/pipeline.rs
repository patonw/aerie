use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// A single step in a workflow
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone)]
pub struct Workstep {
    #[serde(default)]
    pub disabled: bool,

    #[serde(default)]
    pub temperature: Option<f64>,

    /// Override the workflow preamble for this step
    #[serde(default)]
    pub preamble: Option<String>,

    /// Include the last `N` messages as context
    #[serde(default)]
    pub depth: Option<usize>,

    // TODO: templating mechanism
    pub prompt: String,

    pub tools: Option<String>,
}

/// A sequence of steps consisting of LLM invocations
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Pipeline {
    pub name: String,

    /// Only retain the final response in the chat history
    #[serde(default)]
    pub collapse: bool,

    /// Override the global preamble
    #[serde(default)]
    pub preamble: Option<String>,

    pub steps: Vec<Workstep>,
}

impl Default for Pipeline {
    fn default() -> Self {
        Self {
            name: Default::default(),
            collapse: false,
            preamble: None,
            steps: vec![Workstep {
                disabled: false,
                temperature: None,
                preamble: None,
                depth: None,
                prompt: "{{user_prompt}}".to_string(),
                tools: None,
            }],
        }
    }
}
