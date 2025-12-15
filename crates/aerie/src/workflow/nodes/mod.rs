use delegate::delegate;
use kinded::Kinded;
use serde::{Deserialize, Serialize};

pub mod agent;
pub mod chat;
pub mod history;
pub mod json;
pub mod misc;
pub mod primatives;
pub mod scaffold;

pub use agent::*;
pub use chat::*;
pub use history::*;
pub use json::*;
pub use misc::*;
pub use primatives::*;
pub use scaffold::*;

pub const MIN_WIDTH: f32 = 128.0;
pub const MIN_HEIGHT: f32 = 32.0;

use crate::workflow::WorkflowError;

pub use super::{DynNode, EditContext, RunContext, UiNode, Value, ValueKind};

// Allows us to just return the delegatee instead of calling a method on it.
// i.e. instead of delegated call like `self.0.foo()` just return `self.0`
trait NoopExt {
    fn noop(self) -> Self;
}

impl<T> NoopExt for T {
    fn noop(self) -> Self {
        self
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Serialize, Deserialize, Kinded)]
pub enum WorkNode {
    Comment(CommentNode),
    Preview(Preview),
    Text(Text),
    Tools(Tools),
    Start(Start),
    Fallback(Fallback),
    Select(Select),
    Finish(Finish),
    Panic(Panic),
    Demote(Demote),
    CreateMessage(CreateMessage),
    GraftChat(GraftHistory),
    MaskChat(MaskHistory),
    ExtendHistory(ExtendHistory),
    Agent(AgentNode),
    Context(ChatContext),
    Chat(ChatNode),
    Structured(StructuredChat),
    InvokeTool(InvokeTool),
    ParseJson(ParseJson),
    ValidateJson(ValidateJson),
    TransformJson(TransformJson),
    TemplateNode(TemplateNode),
    GatherJson(GatherJson),
}

impl WorkNode {
    delegate! {
        to match self {
            WorkNode::Comment(node) => node,
            WorkNode::Preview(node) => node,
            WorkNode::Text(node) => node,
            WorkNode::Tools(node) => node,
            WorkNode::Start(node) => node,
            WorkNode::Fallback(node) => node,
            WorkNode::Select(node) => node,
            WorkNode::Finish(node) => node,
            WorkNode::Panic(node) => node,
            WorkNode::Demote(node) => node,
            WorkNode::CreateMessage(node) => node,
            WorkNode::GraftChat(node) => node,
            WorkNode::MaskChat(node) => node,
            WorkNode::ExtendHistory(node) => node,
            WorkNode::Agent(node) => node,
            WorkNode::Context(node) => node,
            WorkNode::Chat(node) => node,
            WorkNode::Structured(node) => node,
            WorkNode::InvokeTool(node) => node,
            WorkNode::ParseJson(node) => node,
            WorkNode::ValidateJson(node) => node,
            WorkNode::TransformJson(node) => node,
            WorkNode::TemplateNode(node) => node,
            WorkNode::GatherJson(node) => node,
        } {
            #[call(noop)]
            pub fn as_dyn(&self) -> &dyn DynNode;

            #[call(noop)]
            pub fn as_dyn_mut(&mut self) -> &mut dyn DynNode;

            #[call(noop)]
            pub fn as_ui_mut(&mut self) -> &mut dyn UiNode;

            #[call(noop)]
            pub fn as_ui(&self) -> &dyn UiNode;

            #[deprecated="Switch to call instead"]
            pub async fn forward(&mut self, ctx: &RunContext, inputs: Vec<Option<Value>>) -> Result<(), WorkflowError>;
        }
    }

    pub async fn call(
        &mut self,
        ctx: &RunContext,
        inputs: Vec<Option<Value>>,
    ) -> Result<Vec<Value>, WorkflowError> {
        match self {
            WorkNode::Fallback(node) => node.call(ctx, inputs).await,
            WorkNode::Select(node) => node.call(ctx, inputs).await,
            WorkNode::Demote(node) => node.call(ctx, inputs).await,
            _ => {
                #[allow(deprecated)]
                self.forward(ctx, inputs).await?;

                let outputs = (0..self.as_dyn().outputs())
                    .map(|i| self.as_dyn().value(i))
                    .collect::<Vec<_>>();

                Ok(outputs)
            }
        }
    }
}
