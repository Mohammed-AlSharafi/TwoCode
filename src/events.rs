use crate::tools::Tool;

#[derive(Debug, Clone)]
pub enum DisplayEvent {
    Reasoning(String),
    Content(String),
    ToolCall(Tool),
    Error(String)
}