//! The ticket desk: the deterministic core of the ticket API (#2). It holds every ticket the tenant's users and
//! agents reported, from the acknowledgement until the fix reaches their deployment, and it alone moves a ticket's
//! state: forward only, and only on a signal (the report's validation, #4, or the Admiral, #3). Nothing here judges
//! a report resolved on its own, and there is no state that says "everything is fine".
//!
//! Tickets are held in memory: an instance that restarts starts with none (PROPOSALS.md, a durable ticket store).

use std::collections::HashMap;
use std::sync::Mutex;

use skipper_model::types::services::skipper::{Ticket, TicketEvent, TicketState};
use tokio::sync::watch;

/// One hour, the rate limit's period.
const HOUR_MS: i64 = 60 * 60 * 1000;

/// The default of `Config.reports_per_hour` (proto/services/skipper.proto), until the first config arrives.
pub const DEFAULT_REPORTS_PER_HOUR: i32 = 10;

/// What a reporter sends: `CreateTicketRequest` without the routing and the reporter, which the desk does not
/// take from a request. Its details and service are the report's validation's (#4) to read.
#[derive(Debug, Default, Clone)]
#[allow(dead_code)]
pub struct Report {
    pub summary: String,
    pub details: String,
    pub service: String,
    pub client_request_id: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeskError {
    /// The reporter opened its `reports_per_hour` already; the next ticket is possible in `retry_after_ms`.
    RateLimited { retry_after_ms: i64 },
    /// No such ticket, or not this reporter's: the two are one answer, so a ticket id tells nobody else anything.
    NotFound,
    /// A signal that would move the ticket back, keep it where it is, or name no state a signal may set.
    NotForward { from: TicketState, to: TicketState },
}

/// One reporter's token bucket: `tokens` left at `at_ms`, refilling at `reports_per_hour` an hour.
struct Bucket {
    tokens: f64,
    at_ms: i64,
}

struct Entry {
    reporter: String,
    client_request_id: String,
    // Kept for the report's validation (#4) and the Admiral (#3); the reporter follows the ticket.
    #[allow(dead_code)]
    report: Report,
    updates: watch::Sender<Ticket>,
}

impl Entry {
    fn ticket(&self) -> Ticket {
        self.updates.borrow().clone()
    }
}

#[derive(Default)]
struct Desk {
    tickets: HashMap<String, Entry>,
    buckets: HashMap<String, Bucket>,
}

pub struct TicketDesk {
    reports_per_hour: Mutex<i32>,
    desk: Mutex<Desk>,
}

impl Default for TicketDesk {
    fn default() -> Self {
        Self::new(DEFAULT_REPORTS_PER_HOUR)
    }
}

impl TicketDesk {
    pub fn new(reports_per_hour: i32) -> Self {
        TicketDesk { reports_per_hour: Mutex::new(reports_per_hour.max(1)), desk: Mutex::new(Desk::default()) }
    }

    /// Applies `Config.reports_per_hour`, delivered at start and on every change.
    pub fn set_reports_per_hour(&self, n: i32) {
        *self.reports_per_hour.lock().unwrap() = n.max(1);
    }

    fn reports_per_hour(&self) -> i32 {
        *self.reports_per_hour.lock().unwrap()
    }

    /// Opens a ticket for `reporter` (the verified caller) at `now_ms`, in RECEIVED: the acknowledgement. A report
    /// that repeats a `client_request_id` of the same reporter is a retry and gets that ticket again, costing
    /// nothing. Otherwise it takes one of the reporter's `reports_per_hour`, or is refused.
    pub fn create(&self, reporter: &str, report: Report, now_ms: i64) -> Result<Ticket, DeskError> {
        let per_hour = f64::from(self.reports_per_hour());
        let mut desk = self.desk.lock().unwrap();

        if !report.client_request_id.is_empty() {
            let retried = desk
                .tickets
                .values()
                .find(|e| e.reporter == reporter && e.client_request_id == report.client_request_id);
            if let Some(entry) = retried {
                return Ok(entry.ticket());
            }
        }

        let bucket = desk.buckets.entry(reporter.to_string()).or_insert(Bucket { tokens: per_hour, at_ms: now_ms });
        let elapsed = (now_ms - bucket.at_ms).max(0) as f64;
        bucket.tokens = (bucket.tokens + elapsed * per_hour / HOUR_MS as f64).min(per_hour);
        bucket.at_ms = now_ms.max(bucket.at_ms);
        if bucket.tokens < 1.0 {
            let retry_after_ms = ((1.0 - bucket.tokens) * HOUR_MS as f64 / per_hour).ceil() as i64;
            return Err(DeskError::RateLimited { retry_after_ms });
        }
        bucket.tokens -= 1.0;

        let ticket_id = loop {
            let id = format!("{:016x}", rand::random::<u64>());
            if !desk.tickets.contains_key(&id) {
                break id;
            }
        };
        let ticket = Ticket {
            ticket_id: ticket_id.clone(),
            state: TicketState::Received as i32,
            summary: report.summary.clone(),
            received_at_ms: now_ms,
            updated_at_ms: now_ms,
            history: vec![TicketEvent {
                state: TicketState::Received as i32,
                at_ms: now_ms,
                detail: "report received".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let (updates, _) = watch::channel(ticket.clone());
        desk.tickets.insert(
            ticket_id,
            Entry {
                reporter: reporter.to_string(),
                client_request_id: report.client_request_id.clone(),
                report,
                updates,
            },
        );
        Ok(ticket)
    }

    /// How many tickets the desk holds.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.desk.lock().unwrap().tickets.len()
    }

    /// The ticket `ticket_id`, when `reporter` reported it.
    pub fn get(&self, reporter: &str, ticket_id: &str) -> Result<Ticket, DeskError> {
        let desk = self.desk.lock().unwrap();
        match desk.tickets.get(ticket_id) {
            Some(entry) if entry.reporter == reporter => Ok(entry.ticket()),
            _ => Err(DeskError::NotFound),
        }
    }

    /// The updates of the ticket `ticket_id`, when `reporter` reported it: a receiver holding its current state,
    /// which sees every later one.
    pub fn subscribe(&self, reporter: &str, ticket_id: &str) -> Result<watch::Receiver<Ticket>, DeskError> {
        let desk = self.desk.lock().unwrap();
        match desk.tickets.get(ticket_id) {
            Some(entry) if entry.reporter == reporter => Ok(entry.updates.subscribe()),
            _ => Err(DeskError::NotFound),
        }
    }

    /// Moves the ticket `ticket_id` to `to` on a signal, saying `detail`, and tells its subscribers. Only the
    /// report's validation (#4) and the Admiral's signals (#3) call it: a ticket never moves on the Skipper's own
    /// judgment. It moves forward only (TicketState's order), and never back to RECEIVED or to no state.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn advance(&self, ticket_id: &str, to: TicketState, detail: &str, now_ms: i64) -> Result<Ticket, DeskError> {
        let desk = self.desk.lock().unwrap();
        let entry = desk.tickets.get(ticket_id).ok_or(DeskError::NotFound)?;
        let from = entry.ticket().state();
        if (to as i32) <= (from as i32) || to == TicketState::Unspecified {
            return Err(DeskError::NotForward { from, to });
        }
        entry.updates.send_modify(|t| {
            t.state = to as i32;
            t.updated_at_ms = now_ms;
            t.history.push(TicketEvent { state: to as i32, at_ms: now_ms, detail: detail.to_string(), ..Default::default() });
        });
        Ok(entry.ticket())
    }
}

/// Whether `state` is a ticket's last: its reporter is told nothing after it.
pub fn is_final(state: TicketState) -> bool {
    matches!(state, TicketState::FixDeployed | TicketState::Closed)
}

/// What the reporter is told when its report is taken. It promises the process, never an outcome.
pub fn acknowledgement(ticket: &Ticket) -> String {
    format!(
        "Your report is ticket {}. It will be checked against your tenant's services and filed whatever that \
         finds; you will get each step until the fix reaches your deployment.",
        ticket.ticket_id
    )
}
