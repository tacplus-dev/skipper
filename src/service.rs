//! The Skipper service. The ServiceManager serves what tacplus generates from proto/services/skipper.proto: the
//! ticket API (#2) behind the generated door, which refuses every caller outside this instance's tenant, and its
//! config watch. The stream to the Admiral (#3) comes in its own issue.

use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use servicemanager::auth::Caller;
use skipper_model::s2sm::internal::grpc::servers::services::skipper::tickets::TicketsHandler;
use skipper_model::types::services::skipper::{
    Config, CreateTicketRequest, CreateTicketResponse, GetTicketRequest, SubscribeTicketRequest, Ticket,
};
use tokio::sync::watch;
use tonic::Status;

use crate::tickets::{self, DeskError, Report, TicketDesk};

type TicketStream = Pin<Box<dyn futures::Stream<Item = Result<Ticket, Status>> + Send>>;

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The reporter of a call: the verified caller's subject. The generated wrapper has already bound the request's
/// `reporter` ((spec.principal)) to it and refused a call with no caller; this holds the handler to the same.
/// The door admits this tenant's callers and `/system`'s; a ticket is a tenant's user's or agent's, so a
/// platform caller, in no tenant, has none.
fn reporter(caller: Option<&Caller>, stamped: &str) -> Result<String, Status> {
    let subject = caller
        .and_then(|c| c.token().subject())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Status::unauthenticated("tickets need an authenticated caller"))?;
    if !stamped.is_empty() && stamped != subject {
        return Err(Status::invalid_argument("reporter does not match the authenticated caller"));
    }
    let in_tenant = caller.and_then(|c| c.namespace()).is_some_and(|ns| ns.starts_with("/tenants/"));
    if !in_tenant {
        return Err(Status::permission_denied("tickets are the tenant's users' and agents'; this caller is in no tenant"));
    }
    Ok(subject.to_string())
}

fn status(e: DeskError, ticket_id: &str) -> Status {
    match e {
        DeskError::RateLimited { retry_after_ms } => Status::resource_exhausted(format!(
            "too many reports from this reporter: the next one is possible in {}s",
            (retry_after_ms + 999) / 1000
        )),
        DeskError::NotFound => Status::not_found(format!("no ticket {ticket_id} of yours")),
        DeskError::NotForward { from, to } => {
            Status::failed_precondition(format!("ticket {ticket_id} cannot move from {from:?} to {to:?}"))
        }
    }
}

/// A ticket's updates as a stream: its current state, then each change, ending after its final state (or when
/// the desk drops it). The generated server also ends it at the caller's token expiry; the reporter reopens it
/// and gets the current state first.
fn updates(rx: watch::Receiver<Ticket>) -> TicketStream {
    Box::pin(futures::stream::unfold((Some(rx), true), |(rx, first)| async move {
        let mut rx = rx?;
        if !first && rx.changed().await.is_err() {
            return None;
        }
        let ticket = rx.borrow_and_update().clone();
        let next = if tickets::is_final(ticket.state()) { None } else { Some(rx) };
        Some((Ok::<Ticket, Status>(ticket), (next, false)))
    }))
}

/// The ticket API: Report (CreateTicket), Status (GetTicket) and Updates (SubscribeTicket), on the desk.
/// None of them makes a call of its own, so there is no trace_id to carry on (looper#51).
pub struct TicketsService {
    desk: Arc<TicketDesk>,
}

impl TicketsService {
    pub fn new(desk: Arc<TicketDesk>) -> Self {
        TicketsService { desk }
    }
}

#[async_trait::async_trait]
impl TicketsHandler for TicketsService {
    async fn create_ticket(&self, caller: Option<&Caller>, req: CreateTicketRequest) -> Result<CreateTicketResponse, Status> {
        let reporter = reporter(caller, &req.reporter)?;
        let report = Report {
            summary: req.summary,
            details: req.details,
            service: req.service,
            client_request_id: req.client_request_id,
        };
        let ticket = self.desk.create(&reporter, report, now_ms()).map_err(|e| status(e, ""))?;
        // The ticket id only: the report is the tenant's, and stays out of the log.
        tracing::info!(ticket_id = %ticket.ticket_id, "ticket received");
        Ok(CreateTicketResponse { acknowledgement: tickets::acknowledgement(&ticket), ticket: Some(ticket), ..Default::default() })
    }

    async fn get_ticket(&self, caller: Option<&Caller>, req: GetTicketRequest) -> Result<Ticket, Status> {
        let reporter = reporter(caller, &req.reporter)?;
        self.desk.get(&reporter, &req.ticket_id).map_err(|e| status(e, &req.ticket_id))
    }

    async fn subscribe_ticket(
        &self,
        caller: Option<&Caller>,
        req: SubscribeTicketRequest,
    ) -> Result<tonic::Response<TicketStream>, Status> {
        let reporter = reporter(caller, &req.reporter)?;
        let rx = self.desk.subscribe(&reporter, &req.ticket_id).map_err(|e| status(e, &req.ticket_id))?;
        Ok(tonic::Response::new(updates(rx)))
    }
}

/// The lifecycle handler the ServiceManager runs. It holds the ticket desk: no token and no client.
pub struct SkipperHandler {
    desk: Arc<TicketDesk>,
    tickets: Arc<TicketsService>,
}

impl Default for SkipperHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl SkipperHandler {
    pub fn new() -> Self {
        let desk = Arc::new(TicketDesk::default());
        SkipperHandler { tickets: Arc::new(TicketsService::new(desk.clone())), desk }
    }
}

#[async_trait::async_trait]
impl skipper_model::s2sm::handler::Handler for SkipperHandler {
    fn tickets_grpc_impl(&self) -> Arc<dyn TicketsHandler> {
        self.tickets.clone()
    }

    async fn tickets_config(&self, cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
        self.desk.set_reports_per_hour(cfg.reports_per_hour);
        tracing::info!(reports_per_hour = cfg.reports_per_hour, "skipper config applied");
        Ok(())
    }

    async fn shutdown(&self) {
        tracing::info!("skipper shutting down");
    }
}
