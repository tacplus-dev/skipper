//! What the advisor may read, through a trait the core implements and passes in: the advisor never holds a client
//! handle, only this. The advisor names this module, so the LLM-rule gate (scripts/llm-rule-gate.sh) reads it too:
//! it declares the reads and nothing that could hand out a client, a token or a write.

/// The advisor's reads. Each takes the `trace_id` of the request that asked the advisor, so a read the core makes
/// on its behalf stays on that request's trace. Each returns what it read as text for the model (JSON where it is
/// structured), redacted by the core before it gets here.
#[async_trait::async_trait]
pub trait AdvisorReads: Send + Sync {
    /// The tenant's configuration, its redacted twin: no secret values, only their version references.
    async fn redacted_config(&self, trace_id: &str) -> Result<String, ReadError>;

    /// A ticket's status: what was reported, what was checked, where it stands.
    async fn ticket_status(&self, trace_id: &str, ticket_id: &str) -> Result<String, ReadError>;

    /// A ticket's evidence, sanitized: requests, responses, trace ids, log lines, versions.
    async fn ticket_evidence(&self, trace_id: &str, ticket_id: &str) -> Result<String, ReadError>;
}

/// Why a read returned nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// This skipper cannot read it (yet): why.
    Unavailable(String),
    /// No such thing, or not this caller's to read.
    NotFound(String),
    /// The read failed.
    Failed(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Unavailable(why) => write!(f, "unavailable: {why}"),
            ReadError::NotFound(what) => write!(f, "not found: {what}"),
            ReadError::Failed(why) => write!(f, "failed: {why}"),
        }
    }
}

impl std::error::Error for ReadError {}
