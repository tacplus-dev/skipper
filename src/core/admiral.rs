//! The Skipper's one stream to its Admiral (#3), on the Admiral's declared API (tacplus-dev/admiral#3,
//! `admiral.Admiral/BrokerSkipperChannel`). The Skipper dials out (tenant to `/system`, the allowed direction) and
//! holds the stream; the Admiral reaches the Skipper only on it (tacplus-dev/roadmap
//! decisions/skipper-and-admiral-2026-11-01.md, the 2026-11-01 and 2026-11-19 addenda).
//!
//! - Opening: a `SkipperHello` naming each report the Skipper still follows and the last update it holds of it, then
//!   every report revision the Admiral has not accepted yet. A report the Admiral's hello says it does not know is
//!   sent again.
//! - The Admiral's updates move the report's ticket (FILED, FIX_LANDED, FIX_DEPLOYED, CLOSED) on the ticket desk,
//!   each update once, however often it is replayed.
//! - What the Admiral sends is a request: the Skipper decides each one by its own policy (`AdmiralChannel::decide`)
//!   and answers or declines it. It never acts past its own privileges for one.
//!
//! The generated S2S client holds the stream: it reconnects after any end, and calls `opening` on each connect. On a
//! tacplus with make-before-break rotation (tacplus-dev/tacplus#259, #322) it carries the stream across tokens itself;
//! on 2.6 the Admiral's server ends the stream at the token's `exp` and the client reconnects. Either way nothing here
//! changes: the resume cursors and the resent revisions make a reconnect lose nothing.
//!
//! State is held in memory, as the ticket desk's is (PROPOSALS.md, a durable ticket store).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use skipper_model::s2sm::ServiceManager;
use skipper_model::types::services::admiral::admiral::{
    admiral_message, admiral_request, request_answer, skipper_message, AdmiralMessage, AdmiralRequest, Declined,
    Report, ReportCursor, ReportUpdate, RequestAnswer, SkipperHello, SkipperMessage, Stage,
};
use skipper_model::types::services::skipper::TicketState;
use tokio::sync::{mpsc, oneshot};

/// The Skipper's release, told to the Admiral in its hello.
pub const SKIPPER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The most report cursors one hello carries (`SkipperHello.open_reports`' bound).
const MAX_OPEN_REPORTS: usize = 4096;

/// A ticket event's detail bound (`TicketEvent.detail`).
const MAX_DETAIL: usize = 4096;

/// How long a shutdown waits for its goodbye to be handed to the stream.
const GOODBYE_WAIT: Duration = Duration::from_secs(2);

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Where the Admiral's updates move a ticket: the ticket desk, which moves it forward only.
pub trait TicketSignals: Send + Sync {
    /// Moves the ticket `ticket_id` to `to`, saying `detail`; an error when it does not move (not forward, or no
    /// such ticket).
    fn signal(&self, ticket_id: &str, to: TicketState, detail: &str, now_ms: i64) -> Result<(), String>;
}

/// Why a report was not sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// The channel holds this revision of the report, or a newer one.
    StaleRevision { held: u32 },
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::StaleRevision { held } => write!(f, "the channel holds revision {held} of the report already"),
        }
    }
}

impl std::error::Error for SendError {}

/// One report the Skipper sent: its latest revision, as sent (with the trace of the work that caused it), whether the
/// Admiral accepted that revision, and the last update the Skipper holds of it.
struct Tracked {
    message: SkipperMessage,
    revision: u32,
    accepted: bool,
    last_seq: u64,
    /// The Admiral said it is done (FIX_DEPLOYED or CLOSED): no longer named in the hello.
    done: bool,
}

/// A message for the stream, and who waits for it to be handed over.
struct Outgoing {
    message: SkipperMessage,
    handed: Option<oneshot::Sender<()>>,
}

/// The receiving end of what the channel sends outside an answer: reports and the goodbye. `connect` takes it.
pub struct Outbox(mpsc::UnboundedReceiver<Outgoing>);

/// The Skipper's side of the stream.
pub struct AdmiralChannel {
    signals: Arc<dyn TicketSignals>,
    reports: Mutex<HashMap<String, Tracked>>,
    outbox: mpsc::UnboundedSender<Outgoing>,
}

impl AdmiralChannel {
    pub fn new(signals: Arc<dyn TicketSignals>) -> (Arc<Self>, Outbox) {
        let (outbox, rx) = mpsc::unbounded_channel();
        (Arc::new(AdmiralChannel { signals, reports: Mutex::default(), outbox }), Outbox(rx))
    }

    /// What the Skipper sends first on each (re)opened stream: its hello, then every report revision not accepted yet.
    pub fn opening(&self) -> Vec<SkipperMessage> {
        let reports = self.reports.lock().unwrap();
        let mut open: Vec<(&String, &Tracked)> = reports.iter().filter(|(_, t)| !t.done).collect();
        open.sort_by(|a, b| a.0.cmp(b.0));
        let hello = SkipperMessage {
            body: Some(skipper_message::Body::Hello(SkipperHello {
                skipper_version: SKIPPER_VERSION.to_string(),
                open_reports: open
                    .iter()
                    .take(MAX_OPEN_REPORTS)
                    .map(|(id, t)| ReportCursor { report_id: id.to_string(), last_seq: t.last_seq, ..Default::default() })
                    .collect(),
                ..Default::default()
            })),
            // trace: unrelated: the stream's (re)opening is the Skipper's own lifecycle, caused by no request.
            ..Default::default()
        };
        let mut messages = vec![hello];
        messages.extend(open.iter().filter(|(_, t)| !t.accepted).map(|(_, t)| t.message.clone()));
        messages
    }

    /// Sends a revision of a report (its validation's, #4), on the trace of the work that caused it. The channel keeps
    /// it until the Admiral accepts it, and sends it again on every stream until then.
    pub fn send_report(&self, trace_id: &str, report: Report) -> Result<(), SendError> {
        let message = SkipperMessage {
            trace_id: trace_id.to_string(), // propagate
            body: Some(skipper_message::Body::Report(report.clone())),
            ..Default::default()
        };
        {
            let mut reports = self.reports.lock().unwrap();
            if let Some(held) = reports.get(&report.report_id) {
                if held.revision >= report.revision {
                    return Err(SendError::StaleRevision { held: held.revision });
                }
            }
            let (last_seq, done) = reports.get(&report.report_id).map_or((0, false), |t| (t.last_seq, t.done));
            reports.insert(
                report.report_id.clone(),
                Tracked { message: message.clone(), revision: report.revision, accepted: false, last_seq, done },
            );
        }
        let _ = self.outbox.send(Outgoing { message, handed: None });
        Ok(())
    }

    /// Ends the stream on purpose (the Skipper shutting down): its final message carries only `last`, so the Admiral
    /// takes the end as a goodbye, not a fault. Waits a moment for it to be handed to the stream.
    pub async fn goodbye(&self) {
        let (handed, wait) = oneshot::channel();
        // trace: unrelated: the Skipper's shutdown, caused by no request.
        let message = SkipperMessage { last: true, ..Default::default() };
        if self.outbox.send(Outgoing { message, handed: Some(handed) }).is_ok() {
            let _ = tokio::time::timeout(GOODBYE_WAIT, wait).await;
        }
    }

    /// Takes one message from the Admiral and returns what the Skipper sends because of it.
    pub fn receive(&self, message: AdmiralMessage, now_ms: i64) -> Vec<SkipperMessage> {
        if message.last {
            tracing::info!("the Admiral ends the stream on purpose; the client reconnects");
        }
        match message.body {
            Some(admiral_message::Body::Hello(hello)) => {
                tracing::info!(admiral_version = %hello.admiral_version, unknown = hello.unknown_reports.len(), "the Admiral answered the hello");
                let mut reports = self.reports.lock().unwrap();
                let mut again = Vec::new();
                for id in &hello.unknown_reports {
                    if let Some(tracked) = reports.get_mut(id) {
                        tracked.accepted = false;
                        again.push(tracked.message.clone());
                    }
                }
                again
            }
            Some(admiral_message::Body::Accepted(accepted)) => {
                if let Some(t) = self.reports.lock().unwrap().get_mut(&accepted.report_id) {
                    if accepted.revision >= t.revision {
                        t.accepted = true;
                    }
                }
                vec![]
            }
            Some(admiral_message::Body::Update(update)) => {
                self.update(update, now_ms);
                vec![]
            }
            Some(admiral_message::Body::Request(request)) => {
                let answer = RequestAnswer {
                    request_id: request.request_id.clone(),
                    outcome: Some(self.decide(&request)),
                    ..Default::default()
                };
                vec![SkipperMessage {
                    trace_id: message.trace_id.clone(), // propagate
                    body: Some(skipper_message::Body::Answer(answer)),
                    ..Default::default()
                }]
            }
            None => vec![],
        }
    }

    /// One step of a report's progress: each `seq` once, in the order the Admiral numbers them; a replayed or older one
    /// changes nothing. The stages a ticket has a state for move it; the others are the Admiral's (triage, grouping, the
    /// release) and only move the cursor.
    fn update(&self, update: ReportUpdate, now_ms: i64) {
        let mut reports = self.reports.lock().unwrap();
        let Some(tracked) = reports.get_mut(&update.report_id) else {
            tracing::warn!(report_id = %update.report_id, "an update of a report this Skipper did not send");
            return;
        };
        if update.seq <= tracked.last_seq {
            return;
        }
        tracked.last_seq = update.seq;
        let stage = update.stage();
        if matches!(stage, Stage::FixDeployed | Stage::Closed) {
            tracked.done = true;
        }
        drop(reports);
        if let Some((to, detail)) = ticket_signal(&update) {
            if let Err(e) = self.signals.signal(&update.report_id, to, &detail, now_ms) {
                tracing::info!(report_id = %update.report_id, ?stage, error = %e, "the update did not move the ticket");
            }
        }
    }

    /// The Skipper's policy on one request of the Admiral's: it answers only for reports it sent, and only within what
    /// it can do today. It reads no tenant configuration yet (assisted configuration, #6) and reproduces nothing yet
    /// (validation, #4), so it declines those, saying why.
    pub fn decide(&self, request: &AdmiralRequest) -> request_answer::Outcome {
        let report_id = match &request.kind {
            Some(admiral_request::Kind::TenantConfig(r)) => &r.report_id,
            Some(admiral_request::Kind::Reproduce(r)) => &r.report_id,
            None => return declined("a request this Skipper does not know"),
        };
        if !self.reports.lock().unwrap().contains_key(report_id) {
            return declined("no such report from this Skipper");
        }
        match &request.kind {
            Some(admiral_request::Kind::TenantConfig(_)) => {
                declined("this Skipper does not read its tenant's configuration yet")
            }
            _ => declined("this Skipper does not reproduce reports yet"),
        }
    }

    /// Holds the stream to the Admiral through the generated S2S client, which reconnects after any end, until the
    /// process ends. Everything the channel sends goes through it: the opening on each connect, the answers, the
    /// reports and the goodbye.
    pub fn connect(self: Arc<Self>, sm: &ServiceManager, outbox: Outbox) {
        let Outbox(mut outbox) = outbox;
        let (inbound_tx, mut inbound) = mpsc::unbounded_channel::<AdmiralMessage>();
        let opening = self.clone();
        // trace: unrelated: opening the stream is the Skipper's own lifecycle; each message on it carries the trace of
        // what caused it (`receive`, `send_report`).
        let stream = sm.s2s().admiral().admiral_client().broker_skipper_channel(
            move |stream| {
                let messages = opening.opening();
                async move {
                    for message in messages {
                        if let Err(status) = stream.send(message).await {
                            tracing::warn!(%status, "the opening was not sent; the next connect sends it again");
                            return;
                        }
                    }
                }
            },
            Arc::new(move |message: AdmiralMessage| {
                let _ = inbound_tx.send(message);
            }),
        );
        tokio::spawn(async move {
            loop {
                let (messages, handed) = tokio::select! {
                    Some(message) = inbound.recv() => (self.receive(message, now_ms()), None),
                    Some(out) = outbox.recv() => (vec![out.message], out.handed),
                    else => return,
                };
                for message in messages {
                    if let Err(status) = stream.send(message).await {
                        tracing::warn!(%status, "a message to the Admiral was not sent");
                    }
                }
                if let Some(handed) = handed {
                    let _ = handed.send(());
                }
            }
        });
    }
}

fn declined(reason: &str) -> request_answer::Outcome {
    request_answer::Outcome::Declined(Declined { reason: reason.to_string(), ..Default::default() })
}

/// The ticket state an update's stage moves the report's ticket to, and what the reporter is told of it (technical:
/// the advisor tells it to the reporter's audience, PROPOSALS.md).
fn ticket_signal(update: &ReportUpdate) -> Option<(TicketState, String)> {
    let finding = update.finding.as_ref().map(|f| f.summary.as_str()).filter(|s| !s.is_empty());
    let (to, mut detail) = match update.stage() {
        Stage::Filed if update.issue_url.is_empty() => (TicketState::Filed, "filed".to_string()),
        Stage::Filed => (TicketState::Filed, format!("filed as {}", update.issue_url)),
        Stage::FixLanded => (TicketState::FixLanded, "the fix landed".to_string()),
        Stage::FixDeployed if update.release.is_empty() => {
            (TicketState::FixDeployed, "the fix reached your deployment".to_string())
        }
        Stage::FixDeployed => (TicketState::FixDeployed, format!("the fix ({}) reached your deployment", update.release)),
        Stage::Closed => (TicketState::Closed, "closed".to_string()),
        _ => return None,
    };
    if let Some(finding) = finding {
        detail = format!("{detail}: {finding}");
    }
    if detail.len() > MAX_DETAIL {
        let mut end = MAX_DETAIL;
        while !detail.is_char_boundary(end) {
            end -= 1;
        }
        detail.truncate(end);
    }
    Some((to, detail))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use skipper_model::types::services::admiral::admiral::{
        admiral_message, admiral_request, request_answer, skipper_message, AdmiralHello, AdmiralMessage,
        AdmiralRequest, Classification, Finding, Report, ReportAccepted, ReportUpdate, ReproduceRequest,
        Reproduction, Stage, Symptom, TenantConfigRequest,
    };
    use skipper_model::types::services::skipper::TicketState;

    use super::{AdmiralChannel, Outbox, SendError, TicketSignals, SKIPPER_VERSION};

    // The ticket desk as a recorder: every signal it was sent, each moving the ticket.
    #[derive(Default)]
    struct Signals(Mutex<Vec<(String, TicketState, String)>>);

    impl TicketSignals for Signals {
        fn signal(&self, ticket_id: &str, to: TicketState, detail: &str, _now_ms: i64) -> Result<(), String> {
            self.0.lock().unwrap().push((ticket_id.to_string(), to, detail.to_string()));
            Ok(())
        }
    }

    fn channel() -> (Arc<AdmiralChannel>, Outbox, Arc<Signals>) {
        let signals = Arc::new(Signals::default());
        let (channel, outbox) = AdmiralChannel::new(signals.clone());
        (channel, outbox, signals)
    }

    fn report(id: &str, revision: u32) -> Report {
        Report {
            report_id: id.to_string(),
            revision,
            summary: "CreateOrder answers INTERNAL".to_string(),
            symptom: Some(Symptom { service: "orders".to_string(), grpc_code: 13, ..Default::default() }),
            reproduction: Some(Reproduction::default()),
            classification: Some(Classification::default()),
            ..Default::default()
        }
    }

    fn admiral(body: admiral_message::Body) -> AdmiralMessage {
        AdmiralMessage { body: Some(body), trace_id: "trace-admiral-1".to_string(), ..Default::default() }
    }

    fn update(id: &str, seq: u64, stage: Stage) -> AdmiralMessage {
        admiral(admiral_message::Body::Update(ReportUpdate {
            report_id: id.to_string(),
            seq,
            stage: stage as i32,
            ..Default::default()
        }))
    }

    fn accepted(id: &str, revision: u32) -> AdmiralMessage {
        admiral(admiral_message::Body::Accepted(ReportAccepted {
            report_id: id.to_string(),
            revision,
            ..Default::default()
        }))
    }

    fn reports_of(messages: &[super::SkipperMessage]) -> Vec<(String, u32)> {
        messages
            .iter()
            .filter_map(|m| match &m.body {
                Some(skipper_message::Body::Report(r)) => Some((r.report_id.clone(), r.revision)),
                _ => None,
            })
            .collect()
    }

    fn cursors_of(message: &super::SkipperMessage) -> Vec<(String, u64)> {
        match &message.body {
            Some(skipper_message::Body::Hello(hello)) => {
                hello.open_reports.iter().map(|c| (c.report_id.clone(), c.last_seq)).collect()
            }
            other => panic!("not a hello: {other:?}"),
        }
    }

    fn outcome(messages: &[super::SkipperMessage]) -> request_answer::Outcome {
        match &messages[..] {
            [message] => match &message.body {
                Some(skipper_message::Body::Answer(answer)) => answer.outcome.clone().expect("an outcome"),
                other => panic!("not an answer: {other:?}"),
            },
            other => panic!("not one answer: {other:?}"),
        }
    }

    #[test]
    fn a_fresh_stream_opens_with_a_hello_and_nothing_else() {
        let (channel, _, _) = channel();
        let opening = channel.opening();
        assert_eq!(opening.len(), 1);
        match &opening[0].body {
            Some(skipper_message::Body::Hello(hello)) => {
                assert_eq!(hello.skipper_version, SKIPPER_VERSION);
                assert!(hello.open_reports.is_empty());
            }
            other => panic!("not a hello: {other:?}"),
        }
        assert!(!opening[0].last);
    }

    #[tokio::test]
    async fn a_report_is_sent_and_sent_again_on_every_stream_until_accepted() {
        let (channel, mut outbox, _) = channel();
        channel.send_report("trace-report-1", report("t-1", 1)).unwrap();
        let sent = outbox.0.recv().await.expect("the report is sent");
        assert_eq!(reports_of(&[sent.message]), vec![("t-1".to_string(), 1)]);

        // A reconnect: the hello names it, and the unaccepted revision goes again.
        let opening = channel.opening();
        assert_eq!(cursors_of(&opening[0]), vec![("t-1".to_string(), 0)]);
        assert_eq!(reports_of(&opening), vec![("t-1".to_string(), 1)]);

        assert!(channel.receive(accepted("t-1", 1), 1).is_empty());
        let opening = channel.opening();
        assert_eq!(cursors_of(&opening[0]), vec![("t-1".to_string(), 0)]);
        assert!(reports_of(&opening).is_empty(), "an accepted revision is not sent again");
    }

    #[test]
    fn a_newer_revision_waits_for_its_own_acceptance() {
        let (channel, _, _) = channel();
        channel.send_report("t", report("t-1", 1)).unwrap();
        channel.receive(accepted("t-1", 1), 1);
        channel.send_report("t", report("t-1", 2)).unwrap();
        // The acceptance of revision 1, replayed, does not accept revision 2.
        channel.receive(accepted("t-1", 1), 1);
        assert_eq!(reports_of(&channel.opening()), vec![("t-1".to_string(), 2)]);
        assert_eq!(channel.send_report("t", report("t-1", 2)), Err(SendError::StaleRevision { held: 2 }));
        assert_eq!(channel.send_report("t", report("t-1", 1)), Err(SendError::StaleRevision { held: 2 }));
    }

    #[test]
    fn a_report_the_admiral_does_not_know_is_sent_again() {
        let (channel, _, _) = channel();
        channel.send_report("trace-report-1", report("t-1", 1)).unwrap();
        channel.send_report("trace-report-2", report("t-2", 1)).unwrap();
        channel.receive(accepted("t-1", 1), 1);
        channel.receive(accepted("t-2", 1), 1);
        let hello = AdmiralHello {
            admiral_version: "0.1.0".to_string(),
            unknown_reports: vec!["t-2".to_string(), "not-ours".to_string()],
            ..Default::default()
        };
        let again = channel.receive(admiral(admiral_message::Body::Hello(hello)), 1);
        assert_eq!(reports_of(&again), vec![("t-2".to_string(), 1)]);
        // It goes on its own report's trace, not the hello's.
        assert_eq!(again[0].trace_id, "trace-report-2");
        // And it is unaccepted again until the Admiral says otherwise.
        assert_eq!(reports_of(&channel.opening()), vec![("t-2".to_string(), 1)]);
    }

    #[test]
    fn updates_move_the_ticket_once_each_and_the_cursor_resumes_after_them() {
        let (channel, _, signals) = channel();
        channel.send_report("t", report("t-1", 1)).unwrap();
        channel.receive(update("t-1", 1, Stage::Received), 1);
        channel.receive(update("t-1", 2, Stage::Triaged), 1);
        let filed = ReportUpdate {
            report_id: "t-1".to_string(),
            seq: 3,
            stage: Stage::Filed as i32,
            issue_url: "https://github.com/tacplus-dev/gateway/issues/7".to_string(),
            finding: Some(Finding { summary: "the gateway drops the header".to_string(), ..Default::default() }),
            ..Default::default()
        };
        channel.receive(admiral(admiral_message::Body::Update(filed)), 1);
        // A replay after a reconnect changes nothing.
        channel.receive(update("t-1", 2, Stage::Triaged), 1);
        channel.receive(update("t-1", 3, Stage::Filed), 1);
        assert_eq!(cursors_of(&channel.opening()[0]), vec![("t-1".to_string(), 3)]);

        channel.receive(update("t-1", 4, Stage::FixLanded), 1);
        channel.receive(update("t-1", 5, Stage::FixReleased), 1);
        let deployed = ReportUpdate {
            report_id: "t-1".to_string(),
            seq: 6,
            stage: Stage::FixDeployed as i32,
            release: "gateway 2.6.14".to_string(),
            ..Default::default()
        };
        channel.receive(admiral(admiral_message::Body::Update(deployed)), 1);
        // Done: no longer followed, so the hello names it no more.
        assert!(cursors_of(&channel.opening()[0]).is_empty());

        let signals = signals.0.lock().unwrap().clone();
        assert_eq!(
            signals,
            vec![
                (
                    "t-1".to_string(),
                    TicketState::Filed,
                    "filed as https://github.com/tacplus-dev/gateway/issues/7: the gateway drops the header".to_string()
                ),
                ("t-1".to_string(), TicketState::FixLanded, "the fix landed".to_string()),
                ("t-1".to_string(), TicketState::FixDeployed, "the fix (gateway 2.6.14) reached your deployment".to_string()),
            ]
        );
    }

    #[test]
    fn an_update_of_a_report_this_skipper_did_not_send_moves_nothing() {
        let (channel, _, signals) = channel();
        channel.receive(update("t-9", 1, Stage::Closed), 1);
        assert!(signals.0.lock().unwrap().is_empty());
        assert!(cursors_of(&channel.opening()[0]).is_empty());
    }

    #[test]
    fn a_long_finding_is_cut_to_the_tickets_bound() {
        let (channel, _, signals) = channel();
        channel.send_report("t", report("t-1", 1)).unwrap();
        let closed = ReportUpdate {
            report_id: "t-1".to_string(),
            seq: 1,
            stage: Stage::Closed as i32,
            finding: Some(Finding { summary: "é".repeat(4096), ..Default::default() }),
            ..Default::default()
        };
        channel.receive(admiral(admiral_message::Body::Update(closed)), 1);
        let detail = signals.0.lock().unwrap()[0].2.clone();
        assert!(detail.len() <= 4096 && detail.starts_with("closed: é"));
    }

    fn request(id: &str, kind: Option<admiral_request::Kind>) -> AdmiralMessage {
        admiral(admiral_message::Body::Request(AdmiralRequest { request_id: id.to_string(), kind, ..Default::default() }))
    }

    fn declined_because(outcome: request_answer::Outcome) -> String {
        match outcome {
            request_answer::Outcome::Declined(declined) => declined.reason,
            other => panic!("not declined: {other:?}"),
        }
    }

    // The Admiral asks; the Skipper decides. Today it can give neither the tenant's config nor a reproduction, and it
    // answers only for reports it sent itself.
    #[test]
    fn the_skipper_decides_each_request() {
        let (channel, _, _) = channel();
        channel.send_report("t", report("t-1", 1)).unwrap();
        let config = |id: &str| {
            Some(admiral_request::Kind::TenantConfig(TenantConfigRequest { report_id: id.to_string(), ..Default::default() }))
        };
        let reproduce = |id: &str| {
            Some(admiral_request::Kind::Reproduce(ReproduceRequest { report_id: id.to_string(), ..Default::default() }))
        };
        let answer = |m| declined_because(outcome(&channel.receive(m, 1)));
        assert_eq!(answer(request("r-1", config("t-1"))), "this Skipper does not read its tenant's configuration yet");
        assert_eq!(answer(request("r-2", reproduce("t-1"))), "this Skipper does not reproduce reports yet");
        assert_eq!(answer(request("r-3", config("t-2"))), "no such report from this Skipper");
        assert_eq!(answer(request("r-4", reproduce("t-2"))), "no such report from this Skipper");
        assert_eq!(answer(request("r-5", None)), "a request this Skipper does not know");
    }

    // Trace propagation (tacplus-dev/looper#51): the answer is sent because of the Admiral's request, so it goes on the
    // request's trace; a report goes on the trace of the work that sent it, on every stream it is sent on.
    #[tokio::test]
    async fn each_message_carries_the_trace_of_what_caused_it() {
        let (channel, mut outbox, _) = channel();
        let config =
            Some(admiral_request::Kind::TenantConfig(TenantConfigRequest { report_id: "t-1".to_string(), ..Default::default() }));
        let answers = channel.receive(request("r-1", config), 1);
        assert_eq!(answers[0].trace_id, "trace-admiral-1");
        match &answers[0].body {
            Some(skipper_message::Body::Answer(answer)) => assert_eq!(answer.request_id, "r-1"),
            other => panic!("not an answer: {other:?}"),
        }

        channel.send_report("trace-report-1", report("t-1", 1)).unwrap();
        assert_eq!(outbox.0.recv().await.unwrap().message.trace_id, "trace-report-1");
        let opening = channel.opening();
        assert_eq!(opening[1].trace_id, "trace-report-1");
        assert_eq!(opening[0].trace_id, "", "the hello is caused by no request");
    }

    #[tokio::test]
    async fn the_goodbye_carries_only_last() {
        let (channel, mut outbox, _) = channel();
        let goodbye = tokio::spawn({
            let channel = channel.clone();
            async move { channel.goodbye().await }
        });
        let out = outbox.0.recv().await.expect("the goodbye is sent");
        assert!(out.message.last);
        assert!(out.message.body.is_none());
        out.handed.expect("the goodbye waits to be handed over").send(()).unwrap();
        goodbye.await.unwrap();
    }

    #[test]
    fn the_admirals_goodbye_is_answered_with_nothing() {
        let (channel, _, _) = channel();
        let goodbye = AdmiralMessage { last: true, ..Default::default() };
        assert!(channel.receive(goodbye, 1).is_empty());
    }
}
