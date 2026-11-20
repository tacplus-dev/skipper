//! The Skipper service: empty for now. The ServiceManager serves what tacplus generates from
//! proto/services/skipper.proto (its config watch); the ticket API (#2) and the stream to the Admiral (#3)
//! come in their own issues.

/// The lifecycle handler the ServiceManager runs. It holds nothing yet: no state, no token, no client.
#[derive(Default)]
pub struct SkipperHandler;

impl SkipperHandler {
    pub fn new() -> Self {
        SkipperHandler
    }
}

#[async_trait::async_trait]
impl skipper_model::s2sm::handler::Handler for SkipperHandler {
    async fn shutdown(&self) {
        tracing::info!("skipper shutting down");
    }
}
