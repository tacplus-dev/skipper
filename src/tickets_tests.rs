use skipper_model::types::services::skipper::TicketState;

use crate::tickets::{self, DeskError, Report, TicketDesk};

const ALICE: &str = "spiffe://example.com/tenants/acme/users/alice";
const BOB: &str = "spiffe://example.com/tenants/acme/users/bob";
const MINUTE_MS: i64 = 60 * 1000;
const T0: i64 = 1_800_000_000_000;

fn report(summary: &str) -> Report {
    Report { summary: summary.to_string(), ..Default::default() }
}

#[test]
fn a_report_is_acknowledged_received() {
    let desk = TicketDesk::default();
    let ticket = desk.create(ALICE, report("checkout returns 500"), T0).unwrap();
    assert_eq!(ticket.state(), TicketState::Received);
    assert_eq!(ticket.summary, "checkout returns 500");
    assert_eq!(ticket.received_at_ms, T0);
    assert_eq!(ticket.history.len(), 1);
    assert_eq!(ticket.history[0].state(), TicketState::Received);
    assert_eq!(ticket.ticket_id.len(), 16);
    assert!(tickets::acknowledgement(&ticket).contains(&ticket.ticket_id));
    assert_eq!(desk.get(ALICE, &ticket.ticket_id).unwrap(), ticket);
}

#[test]
fn each_report_is_its_own_ticket() {
    let desk = TicketDesk::default();
    let a = desk.create(ALICE, report("one"), T0).unwrap();
    let b = desk.create(ALICE, report("two"), T0).unwrap();
    assert_ne!(a.ticket_id, b.ticket_id);
}

// The acknowledgement promises the process, never an outcome.
#[test]
fn the_acknowledgement_never_says_all_is_well() {
    let desk = TicketDesk::default();
    let ack = tickets::acknowledgement(&desk.create(ALICE, report("x"), T0).unwrap()).to_lowercase();
    for word in ["fine", "resolved", "no problem", "working as", "nothing wrong"] {
        assert!(!ack.contains(word), "the acknowledgement says {word:?}: {ack}");
    }
}

#[test]
fn reports_are_rate_limited_per_reporter() {
    let desk = TicketDesk::new(2);
    desk.create(ALICE, report("1"), T0).unwrap();
    desk.create(ALICE, report("2"), T0).unwrap();
    // The bucket is empty: the next report is possible once half an hour refills one.
    assert_eq!(desk.create(ALICE, report("3"), T0), Err(DeskError::RateLimited { retry_after_ms: 30 * MINUTE_MS }));
    // Another reporter has a bucket of its own.
    desk.create(BOB, report("1"), T0).unwrap();
    // Half an hour later, one more; not two.
    desk.create(ALICE, report("3"), T0 + 30 * MINUTE_MS).unwrap();
    assert!(matches!(desk.create(ALICE, report("4"), T0 + 30 * MINUTE_MS), Err(DeskError::RateLimited { .. })));
}

#[test]
fn the_rate_follows_the_config() {
    let desk = TicketDesk::new(1);
    desk.create(ALICE, report("1"), T0).unwrap();
    assert!(matches!(desk.create(ALICE, report("2"), T0), Err(DeskError::RateLimited { .. })));
    desk.set_reports_per_hour(60);
    // At 60 an hour a minute refills one.
    desk.create(ALICE, report("2"), T0 + MINUTE_MS).unwrap();
}

#[test]
fn a_retry_gets_the_same_ticket_and_costs_nothing() {
    let desk = TicketDesk::new(1);
    let retried = Report { client_request_id: "req-1".to_string(), ..report("once") };
    let first = desk.create(ALICE, retried.clone(), T0).unwrap();
    assert_eq!(desk.create(ALICE, retried.clone(), T0).unwrap(), first);
    // The same id from another reporter is another report.
    let other = desk.create(BOB, retried, T0).unwrap();
    assert_ne!(other.ticket_id, first.ticket_id);
}

#[test]
fn a_ticket_is_its_reporters_alone() {
    let desk = TicketDesk::default();
    let ticket = desk.create(ALICE, report("mine"), T0).unwrap();
    assert_eq!(desk.get(BOB, &ticket.ticket_id), Err(DeskError::NotFound));
    assert!(matches!(desk.subscribe(BOB, &ticket.ticket_id), Err(DeskError::NotFound)));
    assert_eq!(desk.get(ALICE, "0000000000000000"), Err(DeskError::NotFound));
}

#[test]
fn a_ticket_moves_forward_on_signals() {
    let desk = TicketDesk::default();
    let id = desk.create(ALICE, report("x"), T0).unwrap().ticket_id;
    let steps = [
        TicketState::Validating,
        TicketState::NotYetReproduced,
        TicketState::Reproduced,
        TicketState::Filed,
        TicketState::FixLanded,
        TicketState::FixDeployed,
    ];
    for (i, state) in steps.iter().enumerate() {
        let at = T0 + i as i64 + 1;
        let ticket = desk.advance(&id, *state, "signal", at).unwrap();
        assert_eq!(ticket.state(), *state);
        assert_eq!(ticket.updated_at_ms, at);
    }
    let ticket = desk.get(ALICE, &id).unwrap();
    let history: Vec<TicketState> = ticket.history.iter().map(|e| e.state()).collect();
    assert_eq!(history[0], TicketState::Received);
    assert_eq!(&history[1..], &steps);
}

#[test]
fn a_ticket_never_moves_back_or_stays() {
    let desk = TicketDesk::default();
    let id = desk.create(ALICE, report("x"), T0).unwrap().ticket_id;
    desk.advance(&id, TicketState::Reproduced, "reproduced", T0).unwrap();
    for to in [TicketState::NotYetReproduced, TicketState::Reproduced, TicketState::Received, TicketState::Unspecified] {
        assert_eq!(
            desk.advance(&id, to, "back", T0),
            Err(DeskError::NotForward { from: TicketState::Reproduced, to })
        );
    }
    assert_eq!(desk.get(ALICE, &id).unwrap().state(), TicketState::Reproduced);
    assert_eq!(desk.advance("0000000000000000", TicketState::Filed, "", T0), Err(DeskError::NotFound));
}

#[test]
fn only_a_fix_deployed_or_a_close_is_final() {
    assert!(tickets::is_final(TicketState::FixDeployed));
    assert!(tickets::is_final(TicketState::Closed));
    for state in [
        TicketState::Received,
        TicketState::Validating,
        TicketState::NotYetReproduced,
        TicketState::Reproduced,
        TicketState::Filed,
        TicketState::FixLanded,
    ] {
        assert!(!tickets::is_final(state), "{state:?}");
    }
}

// The stream to the Admiral (#3): the Admiral's updates of a report the Skipper sent move the report's ticket on the
// desk, forward only, as every other signal.
#[test]
fn the_admirals_updates_move_the_ticket() {
    use std::sync::Arc;

    use skipper::core::admiral::AdmiralChannel;
    use skipper_model::types::services::admiral::admiral::{
        admiral_message, AdmiralMessage, Classification, Report as AdmiralReport, ReportUpdate, Reproduction, Stage,
        Symptom,
    };

    let desk = Arc::new(TicketDesk::default());
    let id = desk.create(ALICE, report("checkout returns 500"), T0).unwrap().ticket_id;
    let (channel, _outbox) = AdmiralChannel::new(desk.clone());
    let sent = AdmiralReport {
        report_id: id.clone(),
        revision: 1,
        summary: "checkout returns 500".to_string(),
        symptom: Some(Symptom::default()),
        reproduction: Some(Reproduction::default()),
        classification: Some(Classification::default()),
        ..Default::default()
    };
    channel.send_report("trace-1", sent).unwrap();
    let update = |seq: u64, stage: Stage| AdmiralMessage {
        body: Some(admiral_message::Body::Update(ReportUpdate {
            report_id: id.clone(),
            seq,
            stage: stage as i32,
            ..Default::default()
        })),
        ..Default::default()
    };
    channel.receive(update(1, Stage::Filed), T0 + 1);
    channel.receive(update(2, Stage::FixDeployed), T0 + 2);
    // Closed after the fix reached the deployment would move it back: the desk refuses, and the ticket stays.
    channel.receive(update(3, Stage::Closed), T0 + 3);
    let ticket = desk.get(ALICE, &id).unwrap();
    assert_eq!(ticket.state(), TicketState::FixDeployed);
    let states: Vec<TicketState> = ticket.history.iter().map(|e| e.state()).collect();
    assert_eq!(states, vec![TicketState::Received, TicketState::Filed, TicketState::FixDeployed]);
    assert_eq!(ticket.history[1].detail, "filed");
}
