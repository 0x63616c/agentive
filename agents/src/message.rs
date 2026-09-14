use crate::ids::ToolName;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    Text {
        text: String,
    },
    Image {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub role: MessageRole,
    #[serde(default)]
    pub content: Vec<MessageContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<ToolName>,
}

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: vec![MessageContent::Text { text: text.into() }],
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: vec![MessageContent::Text { text: text.into() }],
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: vec![MessageContent::Text { text: text.into() }],
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: Vec::new(),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn tool_result(
        tool_call_id: impl Into<String>,
        name: ToolName,
        content: serde_json::Value,
    ) -> Self {
        Self {
            role: MessageRole::Tool,
            content: vec![MessageContent::Text {
                text: content.to_string(),
            }],
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: Some(name),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: ToolName,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub name: ToolName,
    pub content: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstructionFragment {
    pub key: String,
    pub version: u32,
    pub text: String,
}

impl InstructionFragment {
    pub fn new(key: &'static str, version: u32, text: &'static str) -> Self {
        Self {
            key: key.to_string(),
            version,
            text: text.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompiledInstructions {
    pub core: InstructionFragment,
    pub agent: Option<String>,
    pub run: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompiledRequest {
    pub instructions: CompiledInstructions,
    pub messages: Vec<Message>,
}
