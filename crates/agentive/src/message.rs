use crate::ids::ToolName;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A validated HTTP(S) image URL.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct ImageUrl(String);

impl ImageUrl {
    /// Parses an absolute HTTP(S) URL without embedded credentials.
    pub fn parse(value: impl Into<String>) -> Result<Self, MessageError> {
        let value = value.into();
        let parsed = url::Url::parse(&value).map_err(|_| MessageError::InvalidImageUrl)?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(MessageError::InvalidImageUrl);
        }
        Ok(Self(value))
    }

    /// Returns the validated URL text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ImageUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// Supported portable image media types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageMediaType {
    /// PNG image data.
    Png,
    /// JPEG image data.
    Jpeg,
    /// GIF image data.
    Gif,
    /// WebP image data.
    Webp,
}

impl ImageMediaType {
    /// Parses a supported IANA image media type.
    pub fn parse(value: &str) -> Result<Self, MessageError> {
        match value {
            "image/png" => Ok(Self::Png),
            "image/jpeg" => Ok(Self::Jpeg),
            "image/gif" => Ok(Self::Gif),
            "image/webp" => Ok(Self::Webp),
            _ => Err(MessageError::UnsupportedImageMediaType),
        }
    }

    /// Returns the canonical IANA media type.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }
}

impl Serialize for ImageMediaType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ImageMediaType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// Exactly one validated image source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImageSource {
    /// An image fetched from a validated remote URL.
    Url {
        /// Validated remote image URL.
        url: ImageUrl,
    },
    /// Image data carried directly in the message.
    Inline {
        /// Non-empty encoded image bytes.
        bytes: InlineImageBytes,
    },
}

/// Non-empty inline image bytes.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(transparent)]
pub struct InlineImageBytes(Vec<u8>);

impl InlineImageBytes {
    /// Validates non-empty image bytes.
    pub fn new(bytes: Vec<u8>) -> Result<Self, MessageError> {
        if bytes.is_empty() {
            Err(MessageError::EmptyInlineImage)
        } else {
            Ok(Self(bytes))
        }
    }

    /// Borrows the encoded image bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for InlineImageBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        Self::new(bytes).map_err(serde::de::Error::custom)
    }
}

/// Invalid canonical message content.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum MessageError {
    /// An image URL is not an absolute credential-free HTTP(S) URL.
    #[error("image URL must be an absolute credential-free HTTP(S) URL")]
    InvalidImageUrl,
    /// An image media type is outside the portable supported set.
    #[error("image media type is not supported")]
    UnsupportedImageMediaType,
    /// Inline image data is empty.
    #[error("inline image bytes must not be empty")]
    EmptyInlineImage,
}

/// Role assigned to a canonical conversation message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// Trusted instructions for the model.
    System,
    /// Content supplied by the end user.
    User,
    /// A model response.
    Assistant,
    /// The result of a tool invocation.
    Tool,
}

/// A typed content component in a canonical message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    /// UTF-8 text content.
    Text {
        /// The text value.
        text: String,
    },
    /// Image content with an explicit source and media type.
    Image {
        /// The image location or embedded bytes.
        source: ImageSource,
        /// The image encoding.
        media_type: ImageMediaType,
    },
}

/// A canonical conversation entry, including optional tool correlation metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    /// The sender's role.
    pub role: MessageRole,
    /// Ordered content components.
    #[serde(default)]
    pub content: Vec<MessageContent>,
    /// Tool calls requested by an assistant message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// The tool-call identifier answered by a tool message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// The tool name associated with a tool message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<ToolName>,
}

impl Message {
    /// Creates a system text message.
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: vec![MessageContent::Text { text: text.into() }],
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Creates a user text message.
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: vec![MessageContent::Text { text: text.into() }],
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Creates an assistant text message.
    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: vec![MessageContent::Text { text: text.into() }],
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Creates an assistant message requesting the supplied tool calls.
    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: Vec::new(),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            name: None,
        }
    }

    /// Creates a user image message that references a remote URL.
    ///
    /// Returns an error when the URL or media type is invalid.
    pub fn image_url(
        url: impl Into<String>,
        media_type: impl AsRef<str>,
    ) -> Result<Self, MessageError> {
        Ok(Self {
            role: MessageRole::User,
            content: vec![MessageContent::Image {
                source: ImageSource::Url {
                    url: ImageUrl::parse(url)?,
                },
                media_type: ImageMediaType::parse(media_type.as_ref())?,
            }],
            tool_calls: None,
            tool_call_id: None,
            name: None,
        })
    }

    /// Creates a user image message containing inline bytes.
    ///
    /// Returns an error when the bytes are empty or the media type is invalid.
    pub fn image_inline(bytes: Vec<u8>, media_type: impl AsRef<str>) -> Result<Self, MessageError> {
        Ok(Self {
            role: MessageRole::User,
            content: vec![MessageContent::Image {
                source: ImageSource::Inline {
                    bytes: InlineImageBytes::new(bytes)?,
                },
                media_type: ImageMediaType::parse(media_type.as_ref())?,
            }],
            tool_calls: None,
            tool_call_id: None,
            name: None,
        })
    }

    /// Creates a tool-result message correlated with a prior tool call.
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

    /// Returns whether this message contains an inline image.
    pub fn has_inline_image(&self) -> bool {
        self.content.iter().any(|item| {
            matches!(
                item,
                MessageContent::Image {
                    source: ImageSource::Inline { .. },
                    ..
                }
            )
        })
    }

    /// Returns whether this message references a remote image URL.
    pub fn has_url_image(&self) -> bool {
        self.content.iter().any(|item| {
            matches!(
                item,
                MessageContent::Image {
                    source: ImageSource::Url { .. },
                    ..
                }
            )
        })
    }
}

/// A provider-requested tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    /// Provider-visible identifier for this call.
    pub id: String,
    /// Declared tool name.
    pub name: ToolName,
    /// JSON arguments supplied for the tool.
    pub arguments: serde_json::Value,
}

/// The recorded result of a tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResult {
    /// Identifier of the completed tool call.
    pub tool_call_id: String,
    /// Declared tool name.
    pub name: ToolName,
    /// JSON result value.
    pub content: serde_json::Value,
}

/// A versioned instruction fragment included in a compiled request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstructionFragment {
    /// Stable fragment key.
    pub key: String,
    /// Fragment contract version.
    pub version: u32,
    /// Instruction text.
    pub text: String,
}

impl InstructionFragment {
    /// Creates a versioned instruction fragment from static source text.
    pub fn new(key: &'static str, version: u32, text: &'static str) -> Self {
        Self {
            key: key.to_string(),
            version,
            text: text.to_string(),
        }
    }
}

/// Layered instructions prepared for provider rendering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompiledInstructions {
    /// Required SDK instruction fragment.
    pub core: InstructionFragment,
    /// Optional agent-level instructions.
    pub agent: Option<String>,
    /// Optional run-level instructions.
    pub run: Option<String>,
    /// Active feature instruction fragments.
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

/// The complete canonical request assembled before a provider call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompiledRequest {
    /// Layered instructions for the request.
    pub instructions: CompiledInstructions,
    /// Conversation messages sent with the instructions.
    pub messages: Vec<Message>,
}
