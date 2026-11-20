//! What the advisor may return: a typed proposal, never an action. The core validates a proposal again before it
//! does anything with it, and acts on one only with the user's approval (tacplus-dev/roadmap
//! decisions/skipper-and-admiral-2026-11-01.md, 2026-11-19: an LLM may propose; it never holds a token or a write
//! path). The advisor names this module, so the LLM-rule gate (scripts/llm-rule-gate.sh) reads it too: it holds
//! data and checks only, with no client, no token and no re-export.
//!
//! The ConfigChange proposal joins with assisted configuration (#6).

use serde::{Deserialize, Serialize};

/// The longest ticket id a proposal may carry.
pub const MAX_TICKET_ID_BYTES: usize = 128;
/// The longest rationale a proposal may carry.
pub const MAX_RATIONALE_BYTES: usize = 4096;

/// A typed proposal, as the advisor's `propose` tool takes it: JSON tagged by `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Proposal {
    /// Where the cause of a reported problem lies, with a confidence (the validated report's classification,
    /// decided 2026-11-19).
    Classification {
        ticket_id: String,
        cause: Cause,
        /// 0 to 100.
        confidence_percent: u8,
        /// Why, in the terms of the evidence read.
        rationale: String,
    },
}

/// Where a reported problem's cause lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    /// The tenant's own service code.
    TenantCode,
    /// A platform service.
    PlatformService,
    /// The code tacplus generates.
    Generator,
}

impl Cause {
    /// Every cause, in the order the `propose` tool's schema lists them.
    pub const ALL: [Cause; 3] = [Cause::TenantCode, Cause::PlatformService, Cause::Generator];
}

/// Why a proposal is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    EmptyTicketId,
    TicketIdTooLong(usize),
    ConfidenceOutOfRange(u8),
    EmptyRationale,
    RationaleTooLong(usize),
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rejection::EmptyTicketId => write!(f, "ticket_id is empty"),
            Rejection::TicketIdTooLong(n) => {
                write!(f, "ticket_id is {n} bytes, over the {MAX_TICKET_ID_BYTES}-byte limit")
            }
            Rejection::ConfidenceOutOfRange(n) => write!(f, "confidence_percent is {n}, over 100"),
            Rejection::EmptyRationale => write!(f, "rationale is empty"),
            Rejection::RationaleTooLong(n) => {
                write!(f, "rationale is {n} bytes, over the {MAX_RATIONALE_BYTES}-byte limit")
            }
        }
    }
}

impl std::error::Error for Rejection {}

impl Proposal {
    /// Checks what the proposal can check on its own. What needs state (the ticket exists, the user may approve
    /// it) the core checks where it holds that state.
    pub fn validate(&self) -> Result<(), Rejection> {
        match self {
            Proposal::Classification { ticket_id, confidence_percent, rationale, .. } => {
                if ticket_id.trim().is_empty() {
                    return Err(Rejection::EmptyTicketId);
                }
                if ticket_id.len() > MAX_TICKET_ID_BYTES {
                    return Err(Rejection::TicketIdTooLong(ticket_id.len()));
                }
                if *confidence_percent > 100 {
                    return Err(Rejection::ConfidenceOutOfRange(*confidence_percent));
                }
                if rationale.trim().is_empty() {
                    return Err(Rejection::EmptyRationale);
                }
                if rationale.len() > MAX_RATIONALE_BYTES {
                    return Err(Rejection::RationaleTooLong(rationale.len()));
                }
                Ok(())
            }
        }
    }

    /// The JSON schema of a proposal, the `propose` tool's input.
    pub fn input_schema() -> serde_json::Value {
        let causes: Vec<serde_json::Value> =
            Cause::ALL.iter().map(|c| serde_json::to_value(c).expect("a cause serializes")).collect();
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {"type": "string", "enum": ["classification"]},
                "ticket_id": {"type": "string", "minLength": 1, "maxLength": MAX_TICKET_ID_BYTES},
                "cause": {"type": "string", "enum": causes},
                "confidence_percent": {"type": "integer", "minimum": 0, "maximum": 100},
                "rationale": {"type": "string", "minLength": 1, "maxLength": MAX_RATIONALE_BYTES}
            },
            "required": ["kind", "ticket_id", "cause", "confidence_percent", "rationale"],
            "additionalProperties": false
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::core::proposal::{Cause, Proposal, Rejection, MAX_RATIONALE_BYTES, MAX_TICKET_ID_BYTES};

    fn classification() -> Proposal {
        Proposal::Classification {
            ticket_id: "t-1".to_string(),
            cause: Cause::PlatformService,
            confidence_percent: 80,
            rationale: "the trace fails in the gateway".to_string(),
        }
    }

    #[test]
    fn a_classification_reads_from_its_tagged_json() {
        let json = r#"{"kind":"classification","ticket_id":"t-1","cause":"platform_service",
            "confidence_percent":80,"rationale":"the trace fails in the gateway"}"#;
        let proposal: Proposal = serde_json::from_str(json).expect("parses");
        assert_eq!(proposal, classification());
        assert_eq!(proposal.validate(), Ok(()));
    }

    #[test]
    fn unknown_kinds_fields_and_causes_are_refused() {
        for json in [
            r#"{"kind":"write_config","ticket_id":"t-1"}"#,
            r#"{"kind":"classification","ticket_id":"t-1","cause":"generator","confidence_percent":1,
                "rationale":"r","token":"x"}"#,
            r#"{"kind":"classification","ticket_id":"t-1","cause":"kernel","confidence_percent":1,"rationale":"r"}"#,
            r#"{"kind":"classification","ticket_id":"t-1","cause":"generator","confidence_percent":300,
                "rationale":"r"}"#,
        ] {
            assert!(serde_json::from_str::<Proposal>(json).is_err(), "{json} must not parse");
        }
    }

    #[test]
    fn validate_refuses_what_the_schema_bounds() {
        let with = |f: &dyn Fn(&mut String, &mut u8, &mut String)| {
            let mut p = classification();
            let Proposal::Classification { ticket_id, confidence_percent, rationale, .. } = &mut p;
            f(ticket_id, confidence_percent, rationale);
            p.validate()
        };
        assert_eq!(with(&|t, _, _| *t = " ".into()), Err(Rejection::EmptyTicketId));
        assert_eq!(
            with(&|t, _, _| *t = "x".repeat(MAX_TICKET_ID_BYTES + 1)),
            Err(Rejection::TicketIdTooLong(MAX_TICKET_ID_BYTES + 1))
        );
        assert_eq!(with(&|_, c, _| *c = 101), Err(Rejection::ConfidenceOutOfRange(101)));
        assert_eq!(with(&|_, _, r| *r = String::new()), Err(Rejection::EmptyRationale));
        assert_eq!(
            with(&|_, _, r| *r = "x".repeat(MAX_RATIONALE_BYTES + 1)),
            Err(Rejection::RationaleTooLong(MAX_RATIONALE_BYTES + 1))
        );
        assert_eq!(with(&|_, c, _| *c = 100), Ok(()));
    }

    #[test]
    fn the_schema_lists_every_cause_and_what_serde_requires() {
        let schema = Proposal::input_schema();
        let causes = schema["properties"]["cause"]["enum"].as_array().expect("an enum");
        assert_eq!(causes.len(), Cause::ALL.len());
        for cause in Cause::ALL {
            let name = serde_json::to_value(cause).expect("serializes");
            assert!(causes.contains(&name), "{name} is in the schema");
        }
        // The schema's required fields are exactly those a proposal serializes.
        let mut required: Vec<&str> =
            schema["required"].as_array().expect("required").iter().map(|v| v.as_str().expect("a name")).collect();
        let serialized = serde_json::to_value(classification()).expect("serializes");
        let mut fields: Vec<&str> = serialized.as_object().expect("an object").keys().map(|k| k.as_str()).collect();
        required.sort_unstable();
        fields.sort_unstable();
        assert_eq!(required, fields);
    }
}
