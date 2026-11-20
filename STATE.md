# State: skipper

Last updated: 2026-11-20 by the looper (#3, the stream to the Admiral).

- **Version:** 0.1.0, unreleased. Compiler: tacplus 2.6, pinned to 2.6.13 and its build image by digest
  (`tacplus.lock`).
- **What works:** a tenant-owned tacplus Rust service, `skipper` (registry id 110), on the generated
  `ServiceManager`, with `just gates` (the LLM-rule gate, tacplus build, `cargo build --locked`, `cargo test
  --locked`) passing. The ticket API (`skipper.Tickets`, README): `CreateTicket`, `GetTicket` and `SubscribeTicket`
  through the gateway, for any authenticated user or agent of the tenant, no anonymous reports, rate-limited per
  reporter (`Config.reports_per_hour`), the reporter stamped from the verified token, a ticket visible to its
  reporter alone, its state moving forward only and only on a signal. The LLM rule (#5): one binary with a
  deterministic core (`src/core/`, the only part that may hold a client, a token or a write path; works with no
  model) and an LLM advisor (`src/advisor/`) whose model is reached only through llmproxy (`depends: [llmproxy]`,
  its S2S client declared; default alias `claude-opus-latest`) and whose tools are three reads plus `propose`,
  returning a typed `Proposal` (a `Classification` for now) the core validates again. `scripts/llm-rule-gate.sh`
  (`just llm-rule`, with its 44-case fixture test) fails the build if the advisor's side could reach a state
  client, a token or a write path. Every model call and read carries the asking request's `trace_id`. The stream
  to the Admiral (#3, `src/core/admiral.rs`, `depends: [admiral]`): the Skipper dials out and holds one
  `admiral.Admiral/BrokerSkipperChannel` through the generated S2S client, which reconnects after any end; each opening
  is a hello with the resume cursors plus every unaccepted report revision; the Admiral's updates move the tickets
  (FILED, FIX_LANDED, FIX_DEPLOYED, CLOSED), each once; its requests are decided by the Skipper (declined, with why,
  until #4 and #6); a shutdown ends it with `last`; each message carries the trace of what caused it.
- **Gaps:** nothing sends a report to the Admiral yet: validation (#4) calls `AdmiralChannel::send_report`, so
  tickets stay `RECEIVED` until then. Make-before-break rotation of the stream waits for a tacplus 2.7 release to pin
  (here and in the Admiral); on 2.6.13 the stream ends at each token's `exp` and resumes on reconnect, and the
  generated client retries every 5s, not with growing backoff (PROPOSALS.md). The stream's state is in memory. Tickets are held in memory, lost on a restart (PROPOSALS.md, a
  durable ticket store). No rpc asks the advisor yet, and its reads answer "unavailable" until they read the ticket
  desk and the redacted config (#6) (PROPOSALS.md); the model alias is a constant, not config, and whether llmproxy
  being down holds skipper's readiness is unverified (PROPOSALS.md). No release image or workflow, and no CI
  workflow running `just gates` (PROPOSALS.md). `deny.toml` clarifies the generated crates' license until tacplus
  writes it (tacplus-dev/tacplus#368).
- **Blockers:** none.
