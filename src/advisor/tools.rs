//! The advisor's tool table: three reads and `propose`. A read goes through the trait the core passes in; `propose`
//! only records a typed proposal for the core to validate again, show the user and act on with their approval. No
//! tool writes, and none holds a token.

use crate::advisor::model::{ToolSpec, ToolUse};
use crate::core::proposal::Proposal;
use crate::core::reads::AdvisorReads;

pub const GET_REDACTED_CONFIG: &str = "get_redacted_config";
pub const GET_TICKET_STATUS: &str = "get_ticket_status";
pub const GET_TICKET_EVIDENCE: &str = "get_ticket_evidence";
pub const PROPOSE: &str = "propose";

/// The tools the advisor offers the model.
pub fn specs() -> Vec<ToolSpec> {
    let ticket = serde_json::json!({
        "type": "object",
        "properties": {"ticket_id": {"type": "string", "minLength": 1}},
        "required": ["ticket_id"],
        "additionalProperties": false
    });
    let none = serde_json::json!({"type": "object", "properties": {}, "additionalProperties": false});
    vec![
        spec(
            GET_REDACTED_CONFIG,
            "Read the tenant's configuration, redacted: secret values are never shown, only their version references.",
            &none,
        ),
        spec(GET_TICKET_STATUS, "Read a ticket's status: what was reported, what was checked, where it stands.", &ticket),
        spec(
            GET_TICKET_EVIDENCE,
            "Read a ticket's sanitized evidence: requests, responses, trace ids, log lines, versions.",
            &ticket,
        ),
        spec(
            PROPOSE,
            "Propose a typed action. Nothing is done: the skipper validates the proposal, shows it to the user and \
             acts only with their approval.",
            &Proposal::input_schema(),
        ),
    ]
}

fn spec(name: &str, description: &str, schema: &serde_json::Value) -> ToolSpec {
    ToolSpec { name: name.to_string(), description: description.to_string(), input_schema_json: schema.to_string() }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NoInput {}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TicketInput {
    ticket_id: String,
}

/// Runs one tool call and returns its result for the model. A failure is a result too (`error: ...`), so the model
/// can correct itself; nothing here fails the advisor.
pub async fn dispatch(
    reads: &dyn AdvisorReads,
    trace_id: &str,
    call: &ToolUse,
    proposals: &mut Vec<Proposal>,
) -> String {
    let read = match call.name.as_str() {
        GET_REDACTED_CONFIG => match parse(call) {
            Ok(NoInput {}) => reads.redacted_config(trace_id).await,
            Err(e) => return e,
        },
        GET_TICKET_STATUS => match parse(call) {
            Ok(TicketInput { ticket_id }) => reads.ticket_status(trace_id, &ticket_id).await,
            Err(e) => return e,
        },
        GET_TICKET_EVIDENCE => match parse(call) {
            Ok(TicketInput { ticket_id }) => reads.ticket_evidence(trace_id, &ticket_id).await,
            Err(e) => return e,
        },
        PROPOSE => return propose(call, proposals),
        other => {
            let names: Vec<String> = specs().into_iter().map(|s| s.name).collect();
            return format!("error: there is no tool {other:?}; the tools are {}", names.join(", "));
        }
    };
    match read {
        Ok(text) => text,
        Err(e) => format!("error: {e}"),
    }
}

fn parse<T: serde::de::DeserializeOwned>(call: &ToolUse) -> Result<T, String> {
    serde_json::from_str(&call.input_json).map_err(|e| format!("error: {}'s input does not parse: {e}", call.name))
}

fn propose(call: &ToolUse, proposals: &mut Vec<Proposal>) -> String {
    let proposal: Proposal = match parse(call) {
        Ok(p) => p,
        Err(e) => return e,
    };
    if let Err(rejection) = proposal.validate() {
        return format!("error: the proposal is refused: {rejection}");
    }
    proposals.push(proposal);
    format!(
        "recorded as proposal {}. Nothing has been done: the skipper validates it, shows it to the user and acts \
         only with their approval.",
        proposals.len()
    )
}
