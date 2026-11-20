# Skipper

A tenant's own platform agent in tacplus: one per tenant, in the tenant's namespace, with no privilege in `/system`.

- **Tickets:** a person or an agent of the tenant reports a problem; the Skipper reproduces it against the tenant's own
  services with the tenant's own access, attaches the evidence, classifies it, and sends it to its Admiral over one
  long-lived bidirectional stream that the Skipper opens. It keeps the reporter updated in language that fits them, until
  the fix reaches their deployment.
- **Assisted configuration:** it reads the tenant's redacted config and proposes changes; a change is written to the
  tenant's Stratum only after the user approves it, with that user's on-behalf-of token.
- **The LLM proposes, the core acts:** one binary with a deterministic core (the only part that holds a token or writes,
  and works with no model at all) and an LLM advisor whose tools are read-only plus "propose".

Design: tacplus-dev/roadmap `decisions/skipper-and-admiral-2026-11-01.md`. Rust. Apache-2.0.

## The ticket API

`skipper.Tickets` (`proto/services/skipper.proto`), through the gateway with the tenant's normal sign-in. Any
authenticated user or agent of the tenant may call it; there are no anonymous reports, and every rpc names the
target tenant's tid in `tenant`.

| Rpc | What it does |
|-----|--------------|
| `CreateTicket` | Report a problem: acknowledged at once with a ticket in `RECEIVED`. A retry with the same `client_request_id` gets the same ticket. Rate-limited per reporter (`Config.reports_per_hour`, 10 by default): `RESOURCE_EXHAUSTED` past it. |
| `GetTicket` | The ticket's state and its history. |
| `SubscribeTicket` | The ticket's current state, then each change, until the fix reaches the reporter's deployment (`FIX_DEPLOYED`) or the ticket is closed. |

The reporter is the verified caller (`reporter` is `(spec.principal)`, stamped by the generated wrapper), and a
ticket is its reporter's alone: anyone else gets `NOT_FOUND`. A ticket moves forward only (`RECEIVED`,
`VALIDATING`, `NOT_YET_REPRODUCED`, `REPRODUCED`, `FILED`, `FIX_LANDED`, `FIX_DEPLOYED`, or `CLOSED`), and only on
a signal from the report's validation (#4) or the Admiral (#3): the Skipper never says "everything is fine" on its
own judgment. `CreateTicket` and `GetTicket` are MCP tools too (tacplus publishes every unary user rpc).

## Building

Skipper is a tacplus Rust service (`tacplus.yaml`): the proto package `skipper` (`proto/services/skipper.proto`)
declares it `tenant_owned`, so the generated door refuses every caller outside its instance's tenant. The
compiler line is tacplus 2.6; `tacplus.lock` pins its exact release and build image by digest (`tacplus pin`
moves it, in a commit). Generated code (`target/generated-sources`) is never committed.

```bash
just gates      # tacplus build, then cargo build --locked and cargo test --locked
```

The build pipeline runs the gates, then its security checks; their cargo-deny reads `deny.toml` (looper-scan's
default policy, plus the license of each crate tacplus generates: tacplus-dev/tacplus#368).

Its service id comes from the global registry (tacplus-dev/registry); `tacplus registry --proto-dir proto`
syncs the lock `proto/registry.yaml`.
