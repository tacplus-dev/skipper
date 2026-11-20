use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::advisor::model::{Block, Model, ModelError, ModelReply, ModelRequest, Role, ToolUse};
use crate::advisor::tools::{self, GET_REDACTED_CONFIG, GET_TICKET_EVIDENCE, GET_TICKET_STATUS, PROPOSE};
use crate::advisor::{Advice, Advisor, AdvisorError, DEFAULT_MODEL_ALIAS, MAX_ROUNDS};
use crate::core::proposal::{Cause, Proposal};
use crate::core::reads::{AdvisorReads, ReadError};

const TRACE: &str = "trace-advisor-1";

// A model that answers from a script and records every request it gets.
struct ScriptedModel {
    replies: Mutex<VecDeque<Result<ModelReply, ModelError>>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl ScriptedModel {
    fn new(replies: Vec<Result<ModelReply, ModelError>>) -> Arc<Self> {
        Arc::new(ScriptedModel { replies: Mutex::new(replies.into()), requests: Mutex::new(Vec::new()) })
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl Model for ScriptedModel {
    async fn complete(&self, request: ModelRequest) -> Result<ModelReply, ModelError> {
        self.requests.lock().unwrap().push(request);
        self.replies.lock().unwrap().pop_front().unwrap_or_else(|| Ok(ModelReply::Text("done".to_string())))
    }
}

// Reads that answer from fixed text and record the trace_id and ticket of each call.
#[derive(Default)]
struct RecordingReads {
    calls: Mutex<Vec<(String, String, String)>>,
}

impl RecordingReads {
    fn calls(&self) -> Vec<(String, String, String)> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, read: &str, trace_id: &str, ticket_id: &str) {
        self.calls.lock().unwrap().push((read.to_string(), trace_id.to_string(), ticket_id.to_string()));
    }
}

#[async_trait::async_trait]
impl AdvisorReads for RecordingReads {
    async fn redacted_config(&self, trace_id: &str) -> Result<String, ReadError> {
        self.record("config", trace_id, "");
        Ok(r#"{"domains":["example.com"],"oidc_secret":{"version":"3"}}"#.to_string())
    }

    async fn ticket_status(&self, trace_id: &str, ticket_id: &str) -> Result<String, ReadError> {
        self.record("status", trace_id, ticket_id);
        if ticket_id == "t-1" {
            Ok("reproduced".to_string())
        } else {
            Err(ReadError::NotFound(format!("ticket {ticket_id}")))
        }
    }

    async fn ticket_evidence(&self, trace_id: &str, ticket_id: &str) -> Result<String, ReadError> {
        self.record("evidence", trace_id, ticket_id);
        Ok("500 from the gateway".to_string())
    }
}

fn call(id: &str, name: &str, input_json: &str) -> Result<ModelReply, ModelError> {
    Ok(ModelReply::ToolUse(ToolUse { id: id.to_string(), name: name.to_string(), input_json: input_json.to_string() }))
}

fn text(t: &str) -> Result<ModelReply, ModelError> {
    Ok(ModelReply::Text(t.to_string()))
}

const VALID_PROPOSAL: &str = r#"{"kind":"classification","ticket_id":"t-1","cause":"platform_service",
    "confidence_percent":70,"rationale":"the gateway returns 500"}"#;

fn advisor(model: &Arc<ScriptedModel>, reads: &Arc<RecordingReads>) -> Advisor {
    Advisor::new(model.clone(), reads.clone())
}

// The tool result the model was sent last, in the final request.
fn last_tool_result(model: &ScriptedModel) -> String {
    let requests = model.requests();
    let last = requests.last().expect("a request");
    match last.messages.last().map(|m| (m.role, &m.content[..])) {
        Some((Role::User, [Block::ToolResult { content, .. }])) => content.clone(),
        other => panic!("the last message is not a tool result: {other:?}"),
    }
}

#[tokio::test]
async fn a_plain_answer_takes_one_call_and_no_tool() {
    let model = ScriptedModel::new(vec![text("Your gateway domain is example.com.")]);
    let reads = Arc::new(RecordingReads::default());
    let advice = advisor(&model, &reads).advise(TRACE, "what is my domain?").await.expect("advice");
    assert_eq!(advice, Advice { text: "Your gateway domain is example.com.".to_string(), proposals: vec![] });
    let requests = model.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model_alias, DEFAULT_MODEL_ALIAS);
    assert_eq!(requests[0].messages.len(), 1);
    assert!(reads.calls().is_empty());
}

// Trace propagation (tacplus-dev/looper#51): every call the advisor makes because of the question it was asked
// carries that question's trace_id: each model call and each read.
#[tokio::test]
async fn the_trace_id_arrives_on_every_model_call_and_every_read() {
    let model = ScriptedModel::new(vec![
        call("c1", GET_REDACTED_CONFIG, "{}"),
        call("c2", GET_TICKET_STATUS, r#"{"ticket_id":"t-1"}"#),
        call("c3", GET_TICKET_EVIDENCE, r#"{"ticket_id":"t-1"}"#),
        call("c4", PROPOSE, VALID_PROPOSAL),
        text("It is the gateway."),
    ]);
    let reads = Arc::new(RecordingReads::default());
    let advice = advisor(&model, &reads).advise(TRACE, "why does my login fail?").await.expect("advice");
    assert_eq!(advice.text, "It is the gateway.");

    let requests = model.requests();
    assert_eq!(requests.len(), 5);
    for request in &requests {
        assert_eq!(request.trace_id, TRACE);
    }
    let calls = reads.calls();
    assert_eq!(calls.len(), 3);
    for (read, trace_id, _) in &calls {
        assert_eq!(trace_id, TRACE, "the {read} read carries the trace");
    }
}

#[tokio::test]
async fn reads_reach_the_model_as_tool_results_in_order() {
    let model = ScriptedModel::new(vec![call("c1", GET_TICKET_STATUS, r#"{"ticket_id":"t-1"}"#), text("ok")]);
    let reads = Arc::new(RecordingReads::default());
    advisor(&model, &reads).advise(TRACE, "status of t-1?").await.expect("advice");

    let second = &model.requests()[1];
    assert_eq!(second.messages.len(), 3);
    assert_eq!(second.messages[1].role, Role::Assistant);
    assert!(matches!(&second.messages[1].content[..], [Block::ToolUse(u)] if u.id == "c1"));
    assert_eq!(
        second.messages[2].content,
        vec![Block::ToolResult { tool_use_id: "c1".to_string(), content: "reproduced".to_string() }]
    );
    assert_eq!(reads.calls(), vec![("status".to_string(), TRACE.to_string(), "t-1".to_string())]);
}

#[tokio::test]
async fn a_failed_read_is_a_result_the_model_sees() {
    let model = ScriptedModel::new(vec![call("c1", GET_TICKET_STATUS, r#"{"ticket_id":"t-9"}"#), text("no such ticket")]);
    let reads = Arc::new(RecordingReads::default());
    advisor(&model, &reads).advise(TRACE, "status of t-9?").await.expect("advice");
    assert_eq!(last_tool_result(&model), "error: not found: ticket t-9");
}

#[tokio::test]
async fn propose_records_a_valid_proposal_and_does_nothing_else() {
    let model = ScriptedModel::new(vec![call("c1", PROPOSE, VALID_PROPOSAL), text("I propose a classification.")]);
    let reads = Arc::new(RecordingReads::default());
    let advice = advisor(&model, &reads).advise(TRACE, "classify t-1").await.expect("advice");
    assert_eq!(
        advice.proposals,
        vec![Proposal::Classification {
            ticket_id: "t-1".to_string(),
            cause: Cause::PlatformService,
            confidence_percent: 70,
            rationale: "the gateway returns 500".to_string(),
        }]
    );
    assert!(last_tool_result(&model).starts_with("recorded as proposal 1. Nothing has been done"));
    assert!(reads.calls().is_empty(), "proposing reads nothing and writes nothing");
}

#[tokio::test]
async fn an_invalid_or_malformed_proposal_is_refused_back_to_the_model() {
    let refused = [
        (r#"{"kind":"classification","ticket_id":"t-1","cause":"generator","confidence_percent":70,"rationale":" "}"#,
         "error: the proposal is refused: rationale is empty"),
        (r#"{"kind":"write_config","field":"domains"}"#, "error: propose's input does not parse"),
        ("not json", "error: propose's input does not parse"),
    ];
    for (input, want) in refused {
        let model = ScriptedModel::new(vec![call("c1", PROPOSE, input), text("ok")]);
        let reads = Arc::new(RecordingReads::default());
        let advice = advisor(&model, &reads).advise(TRACE, "classify").await.expect("advice");
        assert!(advice.proposals.is_empty(), "{input} is not recorded");
        let got = last_tool_result(&model);
        assert!(got.starts_with(want), "{input}: got {got:?}, want {want:?}...");
    }
}

#[tokio::test]
async fn an_unknown_tool_or_bad_input_is_an_error_result() {
    for (name, input, want) in [
        ("write_config", "{}", "error: there is no tool \"write_config\"; the tools are get_redacted_config, get_ticket_status, get_ticket_evidence, propose"),
        (GET_TICKET_STATUS, r#"{"ticket":"t-1"}"#, "error: get_ticket_status's input does not parse"),
        (GET_REDACTED_CONFIG, r#"{"tenant":"other"}"#, "error: get_redacted_config's input does not parse"),
    ] {
        let model = ScriptedModel::new(vec![call("c1", name, input), text("ok")]);
        let reads = Arc::new(RecordingReads::default());
        advisor(&model, &reads).advise(TRACE, "q").await.expect("advice");
        let got = last_tool_result(&model);
        assert!(got.starts_with(want), "{name} {input}: got {got:?}");
        assert!(reads.calls().is_empty());
    }
}

#[tokio::test]
async fn a_model_that_never_answers_stops_after_max_rounds() {
    let script = (0..MAX_ROUNDS + 3).map(|i| call(&format!("c{i}"), GET_REDACTED_CONFIG, "{}")).collect();
    let model = ScriptedModel::new(script);
    let reads = Arc::new(RecordingReads::default());
    let err = advisor(&model, &reads).advise(TRACE, "q").await.expect_err("no answer");
    assert_eq!(err, AdvisorError::TooManyRounds);
    assert_eq!(model.requests().len(), MAX_ROUNDS);
}

#[tokio::test]
async fn a_model_error_is_the_advisors_error() {
    let model = ScriptedModel::new(vec![Err(ModelError("llmproxy: unavailable".to_string()))]);
    let reads = Arc::new(RecordingReads::default());
    let err = advisor(&model, &reads).advise(TRACE, "q").await.expect_err("an error");
    assert_eq!(err, AdvisorError::Model(ModelError("llmproxy: unavailable".to_string())));
}

#[tokio::test]
async fn the_model_alias_defaults_to_opus_and_can_be_set() {
    let model = ScriptedModel::new(vec![text("a"), text("b")]);
    let reads = Arc::new(RecordingReads::default());
    let default = advisor(&model, &reads);
    assert_eq!(default.model_alias(), "claude-opus-latest");
    default.advise(TRACE, "q").await.expect("advice");
    advisor(&model, &reads).with_model_alias("claude-fable-latest").advise(TRACE, "q").await.expect("advice");
    let aliases: Vec<String> = model.requests().into_iter().map(|r| r.model_alias).collect();
    assert_eq!(aliases, vec!["claude-opus-latest", "claude-fable-latest"]);
}

// The tool table is the three reads and propose: nothing that writes.
#[test]
fn the_tools_are_the_reads_and_propose() {
    let specs = tools::specs();
    let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec![GET_REDACTED_CONFIG, GET_TICKET_STATUS, GET_TICKET_EVIDENCE, PROPOSE]);
    for spec in &specs {
        let schema: serde_json::Value = serde_json::from_str(&spec.input_schema_json).expect("a JSON schema");
        assert_eq!(schema["type"], "object", "{}'s input is an object", spec.name);
        assert!(!spec.description.is_empty());
    }
}
