//! The model, as the advisor sees it: one completion at a time, with the tools it offers. The core passes in the
//! implementation (the core's llmproxy module, over llmproxy's generated S2S client), so the advisor holds no client and no
//! credential of its own; these types are its side of that seam, in llmproxy's shape.

/// Who said a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

/// A tool call the model asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    /// The tool's input, a JSON object.
    pub input_json: String,
}

/// One block of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Text(String),
    ToolUse(ToolUse),
    ToolResult { tool_use_id: String, content: String },
}

/// One message of the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Block>,
}

/// A tool the model may call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// Its input's JSON schema.
    pub input_schema_json: String,
}

/// One completion request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRequest {
    /// The trace of the request that asked the advisor: every call to the model carries it (field 200 on the wire).
    pub trace_id: String,
    /// The model, by the alias llmproxy serves (never a provider's model id).
    pub model_alias: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: i32,
}

/// The model's answer: its text, or one tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelReply {
    Text(String),
    ToolUse(ToolUse),
}

/// A completion that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError(pub String);

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the model failed: {}", self.0)
    }
}

impl std::error::Error for ModelError {}

/// A model to complete with.
#[async_trait::async_trait]
pub trait Model: Send + Sync {
    async fn complete(&self, request: ModelRequest) -> Result<ModelReply, ModelError>;
}
