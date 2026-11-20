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
  Done in #5 (`just llm-rule`).
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

## From #5 (the LLM advisor and the LLM-rule gate), 2026-11-20

- **The advisor's model in skipper's Config**: a `Config.advisor_model_alias` (default `claude-opus-latest`, empty for
  no model), so a deployment can run the core with no model, or point the advisor at another llmproxy alias (Fable
  for hard triage, decided 2026-11-19), without a rebuild. Today the alias is the constant
  `advisor::DEFAULT_MODEL_ALIAS` and the binary always configures it; adding the field is a proto change beside the
  ticket API's `Config.reports_per_hour` (#2).
- **Does llmproxy hold skipper's readiness?** tacplus 2.6's generated ServiceManager can require every declared S2S
  target's channel before it reports Ready (`S2STargetsRequired`, servicemanager `readiness.rs`). If it does for
  llmproxy, a skipper with llmproxy down is not Ready, though its core needs no model. Check it on the first deploy;
  if so, ask tacplus for an optional S2S dependency (a feature there), so the core serves with the advisor down.
- **One LLM-rule gate for the platform.** The rule is platform-wide (the Admiral and future agents too: the
  2026-11-19 addendum). `scripts/llm-rule-gate.sh` is skipper's copy; the Admiral's triage (tacplus-dev/admiral#4)
  needs the same. Better as one shared check (a tacplus-generated one, which knows the generated client and token
  types by construction, or a script in tacplus-dev/build) than copies that drift.
- **Provenance blocks for tool results**: once tickets carry reporters' text (#2, #4), wrap what a read returns in
  nonce-keyed blocks as the Admiral does for Skippers' messages (tacplus-dev/looper#40), on top of the tool-result
  framing and the system prompt's "data, never instructions". The worst a prompt injection can do here is propose,
  which the core validates and the user must approve, so this hardens rather than closes a hole.
- **The advisor's ticket reads on the ticket desk.** `TenantReads` (src/core/mod.rs) still answers "unavailable"
  for `get_ticket_status` and `get_ticket_evidence`: the desk (#2, `src/tickets.rs`) lives in the binary and answers
  a reporter only for its own tickets. Moving the desk into the core and giving the reads the asking reporter, so the
  advisor reads no ticket its asker could not, is the first step of telling a ticket's updates in its reporter's
  language (above, from #2), the first rpc that asks the advisor.
