use std::sync::Arc;

use skipper_model::s2sm::handler::Handler;
use skipper_model::types::sys::healthreport::healthreport::InstanceStatus;

use crate::service::SkipperHandler;

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
fn empty_service_is_healthy_and_serves_nothing_extra() {
    let handler = SkipperHandler::new();
    assert!(matches!(handler.check_health(), InstanceStatus::Healthy));
    assert!(handler.additional_services().is_none());
}

#[tokio::test]
async fn shutdown_returns() {
    let handler: Arc<dyn Handler> = Arc::new(SkipperHandler::new());
    handler.shutdown().await;
}
