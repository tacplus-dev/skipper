# Proposals: skipper

Loopers propose here; Jody decides, and an accepted proposal becomes a ROADMAP line.

## From #1 (the scaffold), 2026-11-19

- **A release image and its workflow** (Dockerfile, `release.yml`, image gates as llmproxy's justfile has
  them: non-root 65532, base pinned by digest). The scaffold has no image yet; the first deployable change
  needs one. Links: tacplus-dev/llmproxy `justfile`, tacplus docs/USER.md "Images come from the service's
  release".
- **A CI workflow running `just gates`** in the pinned build image (and the planning-files check other
  repositories run), so a pull request shows the gates before the merge train does.
- **The LLM-rule build gate** (#5) lands as a `just` recipe of its own beside `test`, so `just gates` carries it.
- **Delete deny.toml's clarifications** once `tacplus.lock` pins a tacplus whose generated crates declare
  their license (tacplus-dev/tacplus#368, open; 2.6.13 still writes none): until then deny.toml states each
  generated crate's license as Apache-2.0, the SPDX header tacplus.yaml puts on every generated file, as
  tacplus-dev/redo's does.
- **Three unmaintained crates reach skipper only through the generated chassis** (cargo-deny, reported but
  not failing the policy): `chrono` (RUSTSEC-2026-0543, via `logger`), `rustls-pemfile` (RUSTSEC-2025-0134,
  via `httpclient`, `servicemanager`), `unsafe-libyaml` (RUSTSEC-2024-0449, via `serde_yaml` in `grpcconfig`,
  `grpcserver`, the model's types). The fix is tacplus's generator, not this repository: worth a tacplus issue
  if none tracks them.

## From #2 (the ticket API), 2026-11-20

- **A durable ticket store.** Tickets are held in memory (`src/tickets.rs`): an instance that restarts, or a second
  instance of the same tenant, knows none of them, though a ticket must be kept until the fix reaches its reporter's
  deployment. A tenant-owned `(spec.record)` store (the tenant's Stratum), written on every acknowledgement and
  signal, would keep them; it also needs a retention rule (the design keeps a triage record until its issue closes
  plus 90 days). Links: tacplus docs/USER.md "Record / Event / Store / Projection Annotations", "service_store".
- **The report's trace on its way to the Admiral** (#3, #4): the desk keeps the report, not its inbound `trace_id`.
  The validation and the send to the Admiral are caused by the report, so they must carry it (looper#51): keep it
  on the desk's entry, never on the stored ticket (stores clear field 200).
- **The caller's tenant from its `tid` claim** once `tacplus.lock` pins a tacplus that has it (tacplus#284,
  kernel#32): today the handler refuses a caller whose namespace is not under `/tenants/` (the door has already
  held it to this tenant or `/system`); `tid` would say it directly.
- **Updates in the reporter's language** (the 2026-11-18 addendum: a non-technical user, a developer, an agent):
  the ticket carries typed states and each signal's detail; telling them to the audience is the advisor's (#5).
