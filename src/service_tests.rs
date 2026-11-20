use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use futures::StreamExt;
use key::keys;
use servicemanager::auth::Caller;
use skipper_model::s2sm::handler::Handler;
use skipper_model::s2sm::internal::grpc::servers::services::skipper::tickets::{
    Authorizer, TicketsHandler, ValidationWrapper,
};
use skipper_model::types::services::skipper::tickets_server::Tickets;
use skipper_model::types::services::skipper::{
    CreateTicketRequest, GetTicketRequest, SubscribeTicketRequest, TicketState,
};
use skipper_model::types::sys::healthreport::healthreport::InstanceStatus;
use tonic::{Code, Status};

use crate::service::{SkipperHandler, TicketsService};
use crate::tickets::TicketDesk;

// The generated registry of this build: what tacplus made of proto/.
const REGISTRY: &str = include_str!("../target/generated-sources/model/registry.json");

// Finds the package node that declares the proto package `name`, anywhere in the registry's tree.
fn find_package<'a>(node: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    let declares = node
        .get("proto_packages")
        .and_then(|p| p.as_array())
        .is_some_and(|p| p.iter().any(|p| p.as_str() == Some(name)));
    if declares {
        return Some(node);
    }
    node.get("children")?.as_array()?.iter().find_map(|child| find_package(child, name))
}

// Skipper is tenant-scope: one instance per tenant, in that tenant's namespace. tenant_owned is what makes
// the generated door refuse every caller outside the instance's own tenant (tacplus docs/USER.md), and
// every service of the package carries it, the generated ones included.
#[test]
fn skipper_is_tenant_owned() {
    let registry: serde_json::Value = serde_json::from_str(REGISTRY).expect("registry.json parses");
    let package = find_package(&registry, "skipper").expect("the registry describes the skipper package");
    let services = package.get("services").and_then(|s| s.as_array()).expect("skipper has services");
    assert!(!services.is_empty(), "skipper has services");
    for service in services {
        assert_eq!(
            service.get("tenant_owned"),
            Some(&serde_json::Value::Bool(true)),
            "{} must be tenant_owned",
            service["name"]
        );
    }
}

#[test]
fn handler_is_healthy_and_serves_nothing_extra() {
    let handler = SkipperHandler::new();
    assert!(matches!(handler.check_health(), InstanceStatus::Healthy));
    assert!(handler.additional_services().is_none());
}

#[tokio::test]
async fn shutdown_returns() {
    let handler: Arc<dyn Handler> = Arc::new(SkipperHandler::new());
    handler.shutdown().await;
}

// ---------------------------------------------------------------------------
// The ticket API, called as the generated wrapper calls it: with the verified caller.
// ---------------------------------------------------------------------------

/// This instance's tenant: tid aaaaaaaa, namespace /tenants/acme (kernel#23).
const TENANT: &str = "aaaaaaaa";
const TENANT_NS: &str = "/tenants/acme";
const ALICE: &str = "spiffe://example.com/tenants/acme/users/alice";
const BOB: &str = "spiffe://example.com/tenants/acme/users/bob";
const AGENT: &str = "spiffe://example.com/tenants/aaaaaaaa/services/triage-agent";
const PLATFORM: &str = "spiffe://example.com/system/services/admiral";

fn caller(subject: &str, namespace: &str) -> Caller {
    let mut token = token::token::Token::new();
    token.set_subject(subject).unwrap();
    token.set_namespace(namespace).unwrap();
    Caller::new(token, "request-1".to_string())
}

fn create(summary: &str) -> CreateTicketRequest {
    CreateTicketRequest { tenant: TENANT.to_string(), summary: summary.to_string(), ..Default::default() }
}

fn service(reports_per_hour: i32) -> (Arc<TicketDesk>, TicketsService) {
    let desk = Arc::new(TicketDesk::new(reports_per_hour));
    (desk.clone(), TicketsService::new(desk))
}

#[tokio::test]
async fn a_user_and_an_agent_of_the_tenant_may_report() {
    let (_, svc) = service(10);
    for (subject, namespace) in [(ALICE, TENANT_NS), (AGENT, TENANT_NS)] {
        let resp = svc.create_ticket(Some(&caller(subject, namespace)), create("broken")).await.unwrap();
        let ticket = resp.ticket.expect("a ticket");
        assert_eq!(ticket.state(), TicketState::Received);
        assert!(resp.acknowledgement.contains(&ticket.ticket_id));
    }
}

#[tokio::test]
async fn no_anonymous_reports() {
    let (desk, svc) = service(10);
    let err = svc.create_ticket(None, create("anonymous")).await.unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
    assert_eq!(desk.len(), 0);
}

// The door admits /system's callers too; they are no tenant's users or agents, so they report nothing.
#[tokio::test]
async fn a_platform_caller_reports_nothing() {
    let (desk, svc) = service(10);
    let err = svc.create_ticket(Some(&caller(PLATFORM, "/system")), create("x")).await.unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
    assert_eq!(desk.len(), 0);
}

#[tokio::test]
async fn the_reporter_is_the_caller_never_the_request() {
    let (desk, svc) = service(10);
    let forged = CreateTicketRequest { reporter: BOB.to_string(), ..create("as bob") };
    let err = svc.create_ticket(Some(&caller(ALICE, TENANT_NS)), forged).await.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert_eq!(desk.len(), 0);
}

#[tokio::test]
async fn a_reporter_over_its_rate_is_refused() {
    let (_, svc) = service(1);
    let alice = caller(ALICE, TENANT_NS);
    svc.create_ticket(Some(&alice), create("1")).await.unwrap();
    let err = svc.create_ticket(Some(&alice), create("2")).await.unwrap_err();
    assert_eq!(err.code(), Code::ResourceExhausted);
    // Bob is not held to Alice's rate.
    svc.create_ticket(Some(&caller(BOB, TENANT_NS)), create("1")).await.unwrap();
}

#[tokio::test]
async fn status_is_the_reporters_alone() {
    let (desk, svc) = service(10);
    let id = svc.create_ticket(Some(&caller(ALICE, TENANT_NS)), create("x")).await.unwrap().ticket.unwrap().ticket_id;
    desk.advance(&id, TicketState::Validating, "reproducing", 1).unwrap();
    let get = |reporter: &str| GetTicketRequest {
        tenant: TENANT.to_string(),
        reporter: reporter.to_string(),
        ticket_id: id.clone(),
        ..Default::default()
    };
    let ticket = svc.get_ticket(Some(&caller(ALICE, TENANT_NS)), get(ALICE)).await.unwrap();
    assert_eq!(ticket.state(), TicketState::Validating);
    let err = svc.get_ticket(Some(&caller(BOB, TENANT_NS)), get(BOB)).await.unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
}

#[tokio::test]
async fn updates_run_until_the_fix_reaches_the_deployment() {
    let (desk, svc) = service(10);
    let alice = caller(ALICE, TENANT_NS);
    let id = svc.create_ticket(Some(&alice), create("x")).await.unwrap().ticket.unwrap().ticket_id;
    let sub = SubscribeTicketRequest { tenant: TENANT.to_string(), ticket_id: id.clone(), ..Default::default() };
    let mut stream = svc.subscribe_ticket(Some(&alice), sub.clone()).await.unwrap().into_inner();
    assert_eq!(stream.next().await.unwrap().unwrap().state(), TicketState::Received);
    for state in [TicketState::NotYetReproduced, TicketState::Filed, TicketState::FixLanded, TicketState::FixDeployed] {
        desk.advance(&id, state, "signal", 1).unwrap();
        assert_eq!(stream.next().await.unwrap().unwrap().state(), state);
    }
    assert!(stream.next().await.is_none(), "the stream ends once the fix reached the deployment");
    // Reopened, it says where the ticket ended, and ends.
    let mut again = svc.subscribe_ticket(Some(&alice), sub).await.unwrap().into_inner();
    assert_eq!(again.next().await.unwrap().unwrap().state(), TicketState::FixDeployed);
    assert!(again.next().await.is_none());
    // Nobody else follows Alice's ticket.
    let bobs = SubscribeTicketRequest { tenant: TENANT.to_string(), ticket_id: id, ..Default::default() };
    let err = svc.subscribe_ticket(Some(&caller(BOB, TENANT_NS)), bobs).await.err().unwrap();
    assert_eq!(err.code(), Code::NotFound);
}

// ---------------------------------------------------------------------------
// Through the generated wrapper and the generated Authorizer, as the ServiceManager wires them: the door, the
// route check and the (spec.principal) stamp are tacplus's, and these pin that the ticket API declares them.
// ---------------------------------------------------------------------------

const HOST_SUBJECT: &str = "spiffe://example.com/tenants/aaaaaaaa/services/skipper";

/// The ServiceManager's AuthorizerAdapter in what it decides: the generated Authorizer behind the wrapper's trait.
struct RealAuthorizer(Arc<grpcserver::authorizer::Authorizer>);

fn to_status(e: grpcserver::authorizer::AuthorizerError) -> Status {
    match e.code {
        16 => Status::unauthenticated(e.message),
        _ => Status::permission_denied(e.message),
    }
}

#[tonic::async_trait]
impl Authorizer for RealAuthorizer {
    async fn authorize(&self, full_method: &str, authorization: Option<&str>) -> Result<HashMap<String, String>, Status> {
        let method = full_method.rsplit('/').next().unwrap_or(full_method);
        self.0
            .authorize_grpc(full_method, method, authorization, &HashMap::new())
            .await
            .map(|tw| grpcserver::authorizer::Authorizer::identity_claims(&tw))
            .map_err(to_status)
    }
    async fn check_at_or_below(
        &self,
        full_method: &str,
        authorization: Option<&str>,
        target_namespace: &str,
    ) -> Result<HashMap<String, String>, Status> {
        self.0
            .check_caller_at_or_below_method(full_method, authorization, target_namespace)
            .map(|tw| grpcserver::authorizer::Authorizer::identity_claims(&tw))
            .map_err(to_status)
    }
    async fn authenticate(&self, full_method: &str, authorization: Option<&str>) -> Result<HashMap<String, String>, Status> {
        self.0
            .authenticate_caller_method(full_method, authorization)
            .map(|tw| grpcserver::authorizer::Authorizer::identity_claims(&tw))
            .map_err(to_status)
    }
}

fn verifying_jwk(key: &keys::ImmutableKey) -> jwk::ImmutableJwk {
    let x = match key.public_key() {
        keys::PublicKey::Ed25519(ed) => base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(ed.as_bytes()),
        _ => panic!("expected an ed25519 public key"),
    };
    jwk::ImmutableJwk::new(jwk::jwk::Jwk {
        kty: "OKP".to_string(),
        alg: Some("EdDSA".to_string()),
        kid: key.id().to_string(),
        key_use: Some("sig".to_string()),
        n: None,
        e: None,
        crv: Some("Ed25519".to_string()),
        x: Some(x),
        y: None,
        exp: None,
        created: None,
    })
    .expect("build jwk")
}

/// A signed token for `subject` in `namespace`, audience skipper.
fn bearer(key: &keys::ImmutableKey, subject: &str, namespace: &str, sub_type: &str) -> String {
    let exp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64 + 3600;
    let mut tok = token::token::Token::new();
    tok.set_issuer("test-issuer").unwrap();
    tok.set_audience("skipper").unwrap();
    tok.set_expiration(exp).unwrap();
    tok.set_namespace(namespace).unwrap();
    tok.set_sub_type(sub_type).unwrap();
    tok.set_subject(subject).unwrap();
    tok.sign(key).expect("sign");
    format!("Bearer {}", tok.base64().expect("signed token has base64"))
}

/// The instance of tenant acme (tid aaaaaaaa) behind its generated wrapper.
struct Instance {
    wrapper: ValidationWrapper,
    key: keys::ImmutableKey,
    desk: Arc<TicketDesk>,
}

async fn instance() -> Instance {
    let key = keys::generate_ed25519_key().expect("generate ed25519 key");
    let auth = Arc::new(grpcserver::authorizer::Authorizer::with_seeded_jwks(vec![verifying_jwk(&key)]));
    // As the ServiceManager does: the authorizer learns its instance's identity from the instance's own token,
    // and the wrapper's route check is told the same.
    let own = bearer(&key, HOST_SUBJECT, TENANT_NS, "service");
    auth.set_host_namespace_from_token(own.trim_start_matches("Bearer ")).expect("own identity token");
    let desk = Arc::new(TicketDesk::new(10));
    let wrapper = ValidationWrapper::new(Arc::new(TicketsService::new(desk.clone())));
    wrapper.set_host_identity(auth.host_identity());
    wrapper.set_authorize_func(Some(Arc::new(RealAuthorizer(auth.clone())))).await;
    Instance { wrapper, key, desk }
}

impl Instance {
    fn signed<T>(&self, msg: T, subject: &str, namespace: &str) -> tonic::Request<T> {
        let mut r = tonic::Request::new(msg);
        r.metadata_mut().insert("authorization", bearer(&self.key, subject, namespace, "user").parse().unwrap());
        r
    }
}

fn refused<T: std::fmt::Debug>(got: Result<T, Status>, code: Code) {
    match got {
        Err(s) if s.code() == code => {}
        other => panic!("want {code:?}, got {other:?}"),
    }
}

// The reporter is stamped from the verified token: a report names no reporter, and its ticket is the caller's.
#[tokio::test]
async fn wrapper_stamps_the_reporter_from_the_token() {
    let p = instance().await;
    let resp = p.wrapper.create_ticket(p.signed(create("broken"), ALICE, TENANT_NS)).await.unwrap().into_inner();
    let id = resp.ticket.unwrap().ticket_id;
    let get = GetTicketRequest { tenant: TENANT.to_string(), ticket_id: id.clone(), ..Default::default() };
    let ticket = p.wrapper.get_ticket(p.signed(get.clone(), ALICE, TENANT_NS)).await.unwrap().into_inner();
    assert_eq!(ticket.state(), TicketState::Received);
    refused(p.wrapper.get_ticket(p.signed(get, BOB, TENANT_NS)).await, Code::NotFound);
    let sub = SubscribeTicketRequest { tenant: TENANT.to_string(), ticket_id: id, ..Default::default() };
    let mut stream = p.wrapper.subscribe_ticket(p.signed(sub, ALICE, TENANT_NS)).await.unwrap().into_inner();
    assert_eq!(stream.next().await.unwrap().unwrap().state(), TicketState::Received);
}

#[tokio::test]
async fn wrapper_refuses_an_anonymous_report() {
    let p = instance().await;
    refused(p.wrapper.create_ticket(tonic::Request::new(create("anonymous"))).await, Code::Unauthenticated);
    assert_eq!(p.desk.len(), 0);
}

#[tokio::test]
async fn wrapper_refuses_a_forged_reporter() {
    let p = instance().await;
    let forged = CreateTicketRequest { reporter: BOB.to_string(), ..create("as bob") };
    refused(p.wrapper.create_ticket(p.signed(forged, ALICE, TENANT_NS)).await, Code::InvalidArgument);
    assert_eq!(p.desk.len(), 0);
}

#[tokio::test]
async fn wrapper_refuses_another_tenants_caller() {
    let p = instance().await;
    let mallory = "spiffe://example.com/tenants/other/users/mallory";
    refused(p.wrapper.create_ticket(p.signed(create("x"), mallory, "/tenants/other")).await, Code::PermissionDenied);
    assert_eq!(p.desk.len(), 0);
}

#[tokio::test]
async fn wrapper_refuses_a_platform_caller() {
    let p = instance().await;
    refused(p.wrapper.create_ticket(p.signed(create("x"), PLATFORM, "/system")).await, Code::PermissionDenied);
    assert_eq!(p.desk.len(), 0);
}

#[tokio::test]
async fn wrapper_refuses_a_report_routed_elsewhere() {
    let p = instance().await;
    let elsewhere = CreateTicketRequest { tenant: "bbbbbbbb".to_string(), ..create("x") };
    refused(p.wrapper.create_ticket(p.signed(elsewhere, ALICE, TENANT_NS)).await, Code::PermissionDenied);
    let unrouted = CreateTicketRequest { tenant: String::new(), ..create("x") };
    refused(p.wrapper.create_ticket(p.signed(unrouted, ALICE, TENANT_NS)).await, Code::InvalidArgument);
    assert_eq!(p.desk.len(), 0);
}
