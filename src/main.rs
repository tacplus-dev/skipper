mod service;
#[cfg(test)]
mod service_tests;
mod tickets;
#[cfg(test)]
mod tickets_tests;

use std::error::Error;
use std::sync::Arc;

use skipper::core::llmproxy::LlmProxyModel;
use skipper::core::Core;
use skipper_model::s2sm::ServiceManager;

#[tokio::main]
async fn main() {
    // The chassis logger, not a tracing_subscriber of our own: tracing allows one global subscriber per
    // process, and the chassis's is the redacting one.
    if let Err(e) = logger::install_bootstrap() {
        eprintln!("FATAL: could not install the redacting logger: {e}");
        std::process::exit(1);
    }

    if let Err(e) = run().await {
        tracing::error!(error = %e, "skipper failed");
        let mut source = e.source();
        while let Some(cause) = source {
            tracing::error!(cause = %cause, "caused by");
            source = cause.source();
        }
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    // ServiceManager::new() reads BOOTSTRAP_CONFIG, connects to kernel over mTLS and returns once the
    // service has its identity (in its tenant's namespace) and config; sm.run() serves the generated
    // surface, behind the generated door that refuses callers outside this instance's tenant, until shutdown.
    let sm = Arc::new(ServiceManager::new().await?);
    // The core holds the ServiceManager, and with it every client and token; its advisor reaches the model through
    // llmproxy (by default Opus) and gets nothing else (src/advisor, scripts/llm-rule-gate.sh).
    let core = Arc::new(Core::with_model(Arc::new(LlmProxyModel::new(sm.clone()))));
    sm.run(Arc::new(service::SkipperHandler::new(core))).await
}
