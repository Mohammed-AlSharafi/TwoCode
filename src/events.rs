use crate::tools::Tool;

#[derive(Debug, Clone)]
pub enum DisplayEvent {
    User(String),
    Reasoning(String),
    Content(String),
    ToolCall(Tool),
    Error(String)
}