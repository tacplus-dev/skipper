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
