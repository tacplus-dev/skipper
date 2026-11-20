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
