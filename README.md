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
