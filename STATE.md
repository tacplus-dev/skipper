# State: skipper

Last updated: 2026-11-20 by the looper (#5, the LLM advisor and the LLM-rule gate).

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
  client, a token or a write path. Every model call and read carries the asking request's `trace_id`.
- **Gaps:** nothing sends a ticket's signals yet: tickets stay `RECEIVED` until the validation (#4) and the stream
  to the Admiral (#3) call the desk's `advance`. Tickets are held in memory, lost on a restart (PROPOSALS.md, a
  durable ticket store). No rpc asks the advisor yet, and its reads answer "unavailable" until they read the ticket
  desk and the redacted config (#6) (PROPOSALS.md); the model alias is a constant, not config, and whether llmproxy
  being down holds skipper's readiness is unverified (PROPOSALS.md). No release image or workflow, and no CI
  workflow running `just gates` (PROPOSALS.md). `deny.toml` clarifies the generated crates' license until tacplus
  writes it (tacplus-dev/tacplus#368).
- **Blockers:** none.
