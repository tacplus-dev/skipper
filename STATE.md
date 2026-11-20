# State: skipper

Last updated: 2026-11-20 by the looper (#2, the ticket API).

- **Version:** 0.1.0, unreleased. Compiler: tacplus 2.6, pinned to 2.6.13 and its build image by digest
  (`tacplus.lock`).
- **What works:** a tenant-owned tacplus Rust service, `skipper` (registry id 110), on the generated
  `ServiceManager`, with `just gates` (tacplus build, `cargo build --locked`, `cargo test --locked`) passing. The
  ticket API (`skipper.Tickets`, README): `CreateTicket`, `GetTicket` and `SubscribeTicket` through the gateway,
  for any authenticated user or agent of the tenant, no anonymous reports, rate-limited per reporter
  (`Config.reports_per_hour`), the reporter stamped from the verified token, a ticket visible to its reporter alone,
  its state moving forward only and only on a signal.
- **Gaps:** nothing sends a ticket's signals yet: tickets stay `RECEIVED` until the validation (#4) and the stream
  to the Admiral (#3) call the desk's `advance`. Tickets are held in memory, lost on a restart (PROPOSALS.md, a
  durable ticket store). No release image or workflow, and no CI workflow running `just gates` (PROPOSALS.md).
  `deny.toml` clarifies the generated crates' license until tacplus writes it (tacplus-dev/tacplus#368).
- **Blockers:** none.
