//! The advisor's model, through llmproxy (tacplus-dev/llmproxy): the platform's one holder of a model credential.
//! The core holds the ServiceManager whose generated S2S client calls it; the advisor gets only the `Model`
//! `LlmProxyModel` implements, so it never holds the client, nor the service token the client mints with.

use std::sync::Arc;

use skipper_model::s2sm::ServiceManager;
use skipper_model::types::services::llmproxy::llmproxy as wire;

use crate::advisor::model::{Block, Message, Model, ModelError, ModelReply, ModelRequest, Role, ToolSpec, ToolUse};

/// The one llmproxy rpc the advisor's model uses, `GenerateCompletionAsService`: the calling service's own identity
/// authorizes it, and a tool call comes back to the advisor, which runs it itself. The ServiceManager's generated S2S
/// client implements it in the binary.
#[async_trait::async_trait]
pub trait LlmProxyClient: Send + Sync {
    /// Sends `request` on the trace `trace_id`.
    async fn generate_completion_as_service(
        &self,
        trace_id: &str,
        request: wire::GenerateCompletionRequest,
    ) -> Result<wire::GenerateCompletionResponse, ModelError>;
}

#[async_trait::async_trait]
impl LlmProxyClient for ServiceManager {
    async fn generate_completion_as_service(
        &self,
        trace_id: &str,
        request: wire::GenerateCompletionRequest,
    ) -> Result<wire::GenerateCompletionResponse, ModelError> {
        let request = wire::GenerateCompletionRequest { trace_id: trace_id.to_string(), ..request }; // propagate
        self.s2s()
            .llmproxy()
            .llm_proxy_client()
            .generate_completion_as_service(request)
            .await
            .map(|response| response.into_inner())
            .map_err(|status| ModelError(format!("llmproxy: {status}")))
    }
}

/// The advisor's model, over llmproxy.
pub struct LlmProxyModel {
    client: Arc<dyn LlmProxyClient>,
}

impl LlmProxyModel {
    pub fn new(client: Arc<dyn LlmProxyClient>) -> Self {
        LlmProxyModel { client }
    }
}

#[async_trait::async_trait]
impl Model for LlmProxyModel {
    async fn complete(&self, request: ModelRequest) -> Result<ModelReply, ModelError> {
        // The call is made because of the request the advisor is answering: it goes on that request's trace.
        let response = self.client.generate_completion_as_service(&request.trace_id, to_wire(&request)).await?;
        from_wire(response)
    }
}

/// The request on the wire, but for its trace, which `LlmProxyClient` sets.
pub fn to_wire(request: &ModelRequest) -> wire::GenerateCompletionRequest {
    wire::GenerateCompletionRequest {
        model_alias: request.model_alias.clone(),
        system: request.system.clone(),
        messages: request.messages.iter().map(message_to_wire).collect(),
        tools: request.tools.iter().map(tool_to_wire).collect(),
        max_tokens: request.max_tokens,
        ..Default::default()
    }
}

fn message_to_wire(message: &Message) -> wire::Message {
    let role = match message.role {
        Role::User => wire::Role::User,
        Role::Assistant => wire::Role::Assistant,
    };
    wire::Message {
        role: role as i32,
        content: message
            .content
            .iter()
            .map(|block| wire::ContentBlock {
                block: Some(match block {
                    Block::Text(text) => wire::content_block::Block::Text(text.clone()),
                    Block::ToolUse(call) => wire::content_block::Block::ToolUse(tool_use_to_wire(call)),
                    Block::ToolResult { tool_use_id, content } => {
                        wire::content_block::Block::ToolResult(wire::ToolResult {
                            tool_use_id: tool_use_id.clone(),
                            content: content.clone(),
                            ..Default::default()
                        })
                    }
                }),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

fn tool_use_to_wire(call: &ToolUse) -> wire::ToolUse {
    wire::ToolUse {
        id: call.id.clone(),
        name: call.name.clone(),
        input_json: call.input_json.clone(),
        ..Default::default()
    }
}

fn tool_to_wire(tool: &ToolSpec) -> wire::ToolSpec {
    wire::ToolSpec {
        name: tool.name.clone(),
        description: tool.description.clone(),
        input_schema_json: tool.input_schema_json.clone(),
        ..Default::default()
    }
}

/// The model's answer from the wire: its text or one tool call.
pub fn from_wire(response: wire::GenerateCompletionResponse) -> Result<ModelReply, ModelError> {
    match response.result {
        Some(wire::generate_completion_response::Result::Text(text)) => Ok(ModelReply::Text(text)),
        Some(wire::generate_completion_response::Result::ToolUse(call)) => {
            Ok(ModelReply::ToolUse(ToolUse { id: call.id, name: call.name, input_json: call.input_json }))
        }
        None => Err(ModelError("llmproxy returned neither text nor a tool call".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use skipper_model::types::services::llmproxy::llmproxy as wire;

    use crate::advisor::model::{Block, Message, Model, ModelError, ModelReply, ModelRequest, Role, ToolSpec, ToolUse};
    use crate::core::llmproxy::{from_wire, to_wire, LlmProxyClient, LlmProxyModel};

    // llmproxy as a recorder: it keeps each call's trace and request, and answers with text.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<(String, wire::GenerateCompletionRequest)>>);

    #[async_trait::async_trait]
    impl LlmProxyClient for Recorder {
        async fn generate_completion_as_service(
            &self,
            trace_id: &str,
            request: wire::GenerateCompletionRequest,
        ) -> Result<wire::GenerateCompletionResponse, ModelError> {
            self.0.lock().unwrap().push((trace_id.to_string(), request));
            Ok(wire::GenerateCompletionResponse {
                result: Some(wire::generate_completion_response::Result::Text("answer".to_string())),
                ..Default::default()
            })
        }
    }

    fn request() -> ModelRequest {
        let call = ToolUse { id: "c1".to_string(), name: "get_ticket_status".to_string(), input_json: "{}".to_string() };
        ModelRequest {
            trace_id: "trace-llmproxy-1".to_string(),
            model_alias: "claude-opus-latest".to_string(),
            system: "system".to_string(),
            messages: vec![
                Message { role: Role::User, content: vec![Block::Text("why?".to_string())] },
                Message { role: Role::Assistant, content: vec![Block::ToolUse(call)] },
                Message {
                    role: Role::User,
                    content: vec![Block::ToolResult { tool_use_id: "c1".to_string(), content: "open".to_string() }],
                },
            ],
            tools: vec![ToolSpec {
                name: "propose".to_string(),
                description: "d".to_string(),
                input_schema_json: "{}".to_string(),
            }],
            max_tokens: 4096,
        }
    }

    // Trace propagation (tacplus-dev/looper#51): the call to llmproxy goes on the trace the advisor was asked on.
    #[tokio::test]
    async fn the_llmproxy_call_carries_the_trace_id() {
        let recorder = Arc::new(Recorder::default());
        let model = LlmProxyModel::new(recorder.clone());
        let request = request();
        assert_eq!(model.complete(request.clone()).await, Ok(ModelReply::Text("answer".to_string())));
        let calls = recorder.0.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, request.trace_id);
        assert_eq!(calls[0].1, to_wire(&request));
    }

    #[test]
    fn the_request_maps_onto_llmproxys_messages() {
        let wire = to_wire(&request());
        assert_eq!(wire.model_alias, "claude-opus-latest");
        assert_eq!(wire.system, "system");
        assert_eq!(wire.max_tokens, 4096);
        assert_eq!(wire.tools.len(), 1);
        assert_eq!(wire.tools[0].name, "propose");
        let roles: Vec<i32> = wire.messages.iter().map(|m| m.role).collect();
        assert_eq!(roles, vec![wire::Role::User as i32, wire::Role::Assistant as i32, wire::Role::User as i32]);
        assert_eq!(wire.messages[0].content[0].block, Some(wire::content_block::Block::Text("why?".to_string())));
        match &wire.messages[1].content[0].block {
            Some(wire::content_block::Block::ToolUse(call)) => {
                assert_eq!((call.id.as_str(), call.name.as_str()), ("c1", "get_ticket_status"))
            }
            other => panic!("not a tool use: {other:?}"),
        }
        match &wire.messages[2].content[0].block {
            Some(wire::content_block::Block::ToolResult(result)) => {
                assert_eq!((result.tool_use_id.as_str(), result.content.as_str()), ("c1", "open"))
            }
            other => panic!("not a tool result: {other:?}"),
        }
    }

    #[test]
    fn the_response_maps_back_to_text_or_a_tool_call() {
        let text = wire::GenerateCompletionResponse {
            result: Some(wire::generate_completion_response::Result::Text("answer".to_string())),
            ..Default::default()
        };
        assert_eq!(from_wire(text), Ok(ModelReply::Text("answer".to_string())));

        let call = wire::GenerateCompletionResponse {
            result: Some(wire::generate_completion_response::Result::ToolUse(wire::ToolUse {
                id: "c2".to_string(),
                name: "propose".to_string(),
                input_json: "{}".to_string(),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert_eq!(
            from_wire(call),
            Ok(ModelReply::ToolUse(ToolUse {
                id: "c2".to_string(),
                name: "propose".to_string(),
                input_json: "{}".to_string()
            }))
        );

        assert!(from_wire(wire::GenerateCompletionResponse::default()).is_err());
    }
}
