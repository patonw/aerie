use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use typed_builder::TypedBuilder;
use uuid::Uuid;

use crate::{
    config::ConfigState,
    events::{AppEvent, NodeStateEvent, WorkflowEvent},
    toolbox::ToolStore,
    ui::{state::AppData, workflow::ViewStack},
    workflow::{ShadowMeta, Value, Workflow, runner::ExecId},
};

#[derive(Serialize, Deserialize, Debug, Clone)]
// #[serde(tag = "type")]
pub enum ActionData {
    Dispatch(Vec<AppEvent>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientAction {
    pub action: ActionData,
}

#[derive(Debug, Serialize, Deserialize, Clone, TypedBuilder)]
pub struct StateDump {
    #[builder(default)]
    pub app_data: AppData,

    #[builder(default)]
    pub tools: ToolStore,

    #[builder(default)]
    pub workflows: BTreeMap<String, ShadowMeta>,

    #[builder(default)]
    pub prompt: String,

    #[builder(default)]
    pub task_count: u16,
}

#[derive(Debug, Serialize, Deserialize, Clone, TypedBuilder)]
pub struct EditorPatch {
    pub name: String,

    pub baseline: Workflow,

    #[builder(default)]
    pub shadow: Option<Workflow>,

    #[builder(default)]
    pub stack: Option<ViewStack>,
}

#[derive(Debug, Serialize, Deserialize, Clone, TypedBuilder)]
pub struct StatePatch {
    #[builder(default, setter(strip_option))]
    pub app_data: Option<AppData>,

    #[builder(default, setter(strip_option))]
    pub tools: Option<ToolStore>,

    #[builder(default, setter(strip_option))]
    pub workflows: Option<BTreeMap<String, ShadowMeta>>,

    #[builder(default, setter(strip_option))]
    pub prompt: Option<String>,

    #[builder(default, setter(strip_option))]
    pub config_state: Option<ConfigState>,

    #[builder(default, setter(strip_option))]
    pub editor: Option<EditorPatch>,

    #[builder(default)]
    pub previews: Option<Vec<((ExecId, Uuid), Value)>>,
}

#[derive(Debug, Serialize, Deserialize, Clone, TypedBuilder)]
pub struct SwitchData {
    pub name: String,
    pub baseline: Workflow,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UpdateData {
    Dump(StateDump),
    Patch(StatePatch),
    Switch(SwitchData),
    NodeState(Vec<NodeStateEvent>),
    Event(WorkflowEvent),
}

/// Some serialization formats need a top-level object to wrap the enum
#[derive(TypedBuilder, Clone, Debug, Serialize, Deserialize)]
#[skip_serializing_none]
pub struct ClientUpdate {
    pub update: UpdateData,

    #[builder(default, setter(strip_option))]
    pub task_count: Option<u16>,
}
