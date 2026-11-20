//! The deterministic core (#5): the only part of skipper that may hold a client, a token or a write path, and it
//! works with no model at all. It gives the advisor what the advisor may have (the reads, through `AdvisorReads`,
//! and the model, through llmproxy) and takes back only typed proposals, which it validates again before anything
//! is done with them (tacplus-dev/roadmap decisions/skipper-and-admiral-2026-11-01.md, 2026-11-19: an LLM may
//! propose; it never holds a token or a write path).

pub mod llmproxy;
pub mod proposal;
pub mod reads;

use std::sync::Arc;

use crate::advisor::model::Model;
use crate::advisor::{Advice, Advisor, AdvisorError};
use crate::core::proposal::{Proposal, Rejection};
use crate::core::reads::{AdvisorReads, ReadError};

/// The reads the core gives its advisor. It holds none of what they read yet: the ticket desk (#2) is the binary's,
/// and answers a reporter only for its own tickets (PROPOSALS.md, the advisor's ticket reads on the ticket desk); the
/// tenant's redacted config comes with assisted configuration (#6, after tacplus-dev/kernel#35). Each read says so,
/// and the model is told rather than left to guess.
#[derive(Debug, Default)]
pub struct TenantReads;

#[async_trait::async_trait]
impl AdvisorReads for TenantReads {
    async fn redacted_config(&self, _trace_id: &str) -> Result<String, ReadError> {
        Err(ReadError::Unavailable("this skipper does not read the tenant's configuration yet".to_string()))
    }

    async fn ticket_status(&self, _trace_id: &str, _ticket_id: &str) -> Result<String, ReadError> {
        Err(ReadError::Unavailable("the advisor does not read the ticket desk yet".to_string()))
    }

    async fn ticket_evidence(&self, _trace_id: &str, _ticket_id: &str) -> Result<String, ReadError> {
        Err(ReadError::Unavailable("the advisor does not read the ticket desk yet".to_string()))
    }
}

/// Why the core gave no advice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// No model is configured: everything but advice still works.
    NoModel,
    Advisor(AdvisorError),
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreError::NoModel => write!(f, "no model is configured"),
            CoreError::Advisor(e) => write!(f, "the advisor failed: {e}"),
        }
    }
}

impl std::error::Error for CoreError {}

/// The core, with its advisor if a model is configured.
pub struct Core {
    advisor: Option<Advisor>,
}

impl Core {
    /// A core with no model.
    pub fn without_model() -> Self {
        Core { advisor: None }
    }

    /// A core whose advisor asks `model` (by default Opus) and reads through the core's own reads.
    pub fn with_model(model: Arc<dyn Model>) -> Self {
        Core::with_advisor(Advisor::new(model, Arc::new(TenantReads)))
    }

    pub fn with_advisor(advisor: Advisor) -> Self {
        Core { advisor: Some(advisor) }
    }

    pub fn has_advisor(&self) -> bool {
        self.advisor.is_some()
    }

    /// Asks the advisor and returns its answer with the proposals that pass the core's own validation. Nothing is
    /// done with them here: acting on one takes the user's approval.
    pub async fn advise(&self, trace_id: &str, question: &str) -> Result<Advice, CoreError> {
        let advisor = self.advisor.as_ref().ok_or(CoreError::NoModel)?;
        let mut advice = advisor.advise(trace_id, question).await.map_err(CoreError::Advisor)?;
        // The advisor checked them already; the core does not take the advisor's word for it.
        advice.proposals.retain(|proposal| match self.validate(proposal) {
            Ok(()) => true,
            Err(rejection) => {
                tracing::warn!(trace_id, %rejection, "the core refused an advisor's proposal");
                false
            }
        });
        Ok(advice)
    }

    /// Validates a proposal, whoever made it: the advisor, or a user or UI calling the core directly, which needs
    /// no model.
    pub fn validate(&self, proposal: &Proposal) -> Result<(), Rejection> {
        proposal.validate()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::advisor::model::{Model, ModelError, ModelReply, ModelRequest, ToolUse};
    use crate::advisor::tools::{GET_TICKET_STATUS, PROPOSE};
    use crate::core::proposal::{Cause, Proposal, Rejection};
    use crate::core::{Core, CoreError};

    // A model that calls each tool once, in order, then answers; it records the tool results it was sent.
    struct Calls(Mutex<Vec<(&'static str, &'static str)>>, Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl Model for Calls {
        async fn complete(&self, request: ModelRequest) -> Result<ModelReply, ModelError> {
            if let Some(crate::advisor::model::Block::ToolResult { content, .. }) =
                request.messages.last().and_then(|m| m.content.last())
            {
                self.1.lock().unwrap().push(content.clone());
            }
            let next = self.0.lock().unwrap().pop();
            Ok(match next {
                Some((name, input)) => ModelReply::ToolUse(ToolUse {
                    id: name.to_string(),
                    name: name.to_string(),
                    input_json: input.to_string(),
                }),
                None => ModelReply::Text("answered".to_string()),
            })
        }
    }

    fn classification(confidence_percent: u8) -> Proposal {
        Proposal::Classification {
            ticket_id: "t-1".to_string(),
            cause: Cause::TenantCode,
            confidence_percent,
            rationale: "the handler panics".to_string(),
        }
    }

    #[tokio::test]
    async fn the_core_works_with_no_model() {
        let core = Core::without_model();
        assert!(!core.has_advisor());
        assert_eq!(core.advise("trace-1", "q").await, Err(CoreError::NoModel));
        assert_eq!(core.validate(&classification(90)), Ok(()));
        assert_eq!(core.validate(&classification(101)), Err(Rejection::ConfidenceOutOfRange(101)));
    }

    #[tokio::test]
    async fn the_cores_advisor_reads_through_the_core_and_returns_validated_proposals() {
        let proposal = r#"{"kind":"classification","ticket_id":"t-1","cause":"tenant_code","confidence_percent":90,
            "rationale":"the handler panics"}"#;
        // Popped from the end: the status read first, then the proposal.
        let model = Arc::new(Calls(Mutex::new(vec![(PROPOSE, proposal), (GET_TICKET_STATUS, r#"{"ticket_id":"t-1"}"#)]), Mutex::default()));
        let core = Core::with_model(model.clone());
        assert!(core.has_advisor());
        let advice = core.advise("trace-1", "classify t-1").await.expect("advice");
        assert_eq!(advice.text, "answered");
        assert_eq!(advice.proposals, vec![classification(90)]);
        let results = model.1.lock().unwrap().clone();
        assert_eq!(results[0], "error: unavailable: the advisor does not read the ticket desk yet");
        assert!(results[1].starts_with("recorded as proposal 1"));
    }
}
