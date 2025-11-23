use delegate::delegate;
use serde::{Deserialize, Serialize};

pub mod chat;
pub mod primatives;
pub mod scaffold;
pub mod tools;

pub use chat::*;
pub use primatives::*;
pub use scaffold::*;
pub use tools::*;

pub const MIN_WIDTH: f32 = 128.0;
pub const MIN_HEIGHT: f32 = 32.0;

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

#[derive(Debug, Clone, Hash, PartialEq, Serialize, Deserialize)]
pub enum WorkNode {
    Preview(Preview),
    Text(Text),
    Tools(Tools),
    Start(Start),
    LLM(LLM),
}

impl WorkNode {
    delegate! {
        to match self {
            WorkNode::Preview(dummy) => dummy,
            WorkNode::Text(text) => text,
            WorkNode::Tools(tools) => tools,
            WorkNode::Start(node) => node,
            WorkNode::LLM(node) => node,
        } {
            #[call(noop)]
            pub fn as_dyn(&self) -> &dyn DynNode;

            #[call(noop)]
            pub fn as_dyn_mut(&mut self) -> &mut dyn DynNode;

            #[call(noop)]
            pub fn as_ui_mut(&mut self) -> &mut dyn UiNode;

            #[call(noop)]
            pub fn as_ui(&self) -> &dyn UiNode;

            pub async fn forward(&mut self, ctx: &RunContext, inputs: Vec<Option<Value>>) -> Result<(), Vec<String>>;
        }
    }
}
