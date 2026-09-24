use std::collections::BTreeMap;

use footnote::Footnote;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use struct_patch::Patch;
use typed_builder::TypedBuilder;
use uuid::Uuid;

use crate::{
    config::{ConfigState, Preferences},
    events::{AppEvent, NodeStateEvent, WorkflowEvent},
    toolbox::ToolStore,
    ui::{
        state::{AppData, AppDataNote, AppDataPatch},
        workflow::ViewStack,
    },
    workflow::{ShadowMeta, Value, Workflow, runner::ExecId},
};

#[derive(Serialize, Deserialize, Debug, Clone)]
// #[serde(tag = "type")]
pub enum ActionData {
    Checkpoint(StatePatchNote),
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

#[derive(
    Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, TypedBuilder, Footnote, Patch,
)]
#[patch(attribute(derive(Debug, Default, Clone, Serialize, Deserialize)))]
pub struct EditorPatch {
    pub name: String,

    pub baseline: Workflow,

    #[builder(default)]
    pub shadow: Option<Workflow>,

    #[builder(default)]
    pub stack: Option<ViewStack>,
}

// Eventually we'll want to just put Footnote/Patch on AppState and bypass this.
#[derive(Debug, Default, Serialize, Deserialize, Clone, TypedBuilder, Footnote, Patch)]
#[patch(attribute(derive(Debug, Default, Clone, Serialize, Deserialize)))]
pub struct StatePatch {
    pub prefs: Preferences,

    #[patch(nesting)]
    pub app_data: AppData,

    #[builder(default, setter(strip_option))]
    pub tools: Option<ToolStore>,

    #[builder(default, setter(strip_option))]
    pub workflows: Option<BTreeMap<String, ShadowMeta>>,

    #[builder(default, setter(strip_option))]
    pub prompt: Option<String>,

    #[builder(default, setter(strip_option))]
    pub images: Option<im::Vector<String>>,

    #[builder(default, setter(strip_option))]
    pub config_state: Option<ConfigState>,

    #[patch(nesting)]
    pub editor: EditorPatch,

    #[builder(default)]
    pub previews: Option<Vec<((ExecId, Uuid), Value)>>,

    #[builder(default)]
    pub errors: Option<Vec<String>>,
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
    Delta(StatePatchPatch), // TODO: some kind of consistency check
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

impl From<UpdateData> for ClientUpdate {
    fn from(value: UpdateData) -> Self {
        Self::builder().update(value).build()
    }
}
