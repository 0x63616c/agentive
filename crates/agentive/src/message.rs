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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bytes: Option<Vec<u8>>,
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

    pub fn image_url(url: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: vec![MessageContent::Image {
                url: Some(url.into()),
                bytes: None,
                media_type: Some(media_type.into()),
            }],
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn image_inline(bytes: Vec<u8>, media_type: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: vec![MessageContent::Image {
                url: None,
                bytes: Some(bytes),
                media_type: Some(media_type.into()),
            }],
            tool_calls: None,
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

    pub fn has_inline_image(&self) -> bool {
        self.content
            .iter()
            .any(|item| matches!(item, MessageContent::Image { bytes: Some(_), .. }))
    }

    pub fn has_url_image(&self) -> bool {
        self.content
            .iter()
            .any(|item| matches!(item, MessageContent::Image { url: Some(_), .. }))
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<InstructionFragment>,
}

impl CompiledInstructions {
    /// Renders deterministic escaped XML for providers requiring flattened instructions.
    #[must_use]
    pub fn render_xml(&self) -> String {
        let mut output = String::from("<instructions>");
        render_instruction(&mut output, "core", &self.core.text);
        if let Some(agent) = &self.agent {
            render_instruction(&mut output, "agent", agent);
        }
        if let Some(run) = &self.run {
            render_instruction(&mut output, "run", run);
        }
        for feature in &self.features {
            render_instruction(&mut output, &feature.key, &feature.text);
        }
        output.push_str("</instructions>");
        output
    }
}

fn render_instruction(output: &mut String, key: &str, text: &str) {
    output.push_str("<fragment key=\"");
    escape_xml(output, key);
    output.push_str("\">");
    escape_xml(output, text);
    output.push_str("</fragment>");
}

fn escape_xml(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '\"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(character),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompiledRequest {
    pub instructions: CompiledInstructions,
    pub messages: Vec<Message>,
}
