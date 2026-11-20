# State: skipper

Last updated: 2026-11-20 by the looper (#1, the scaffold).

- **Version:** 0.1.0, unreleased. Compiler: tacplus 2.6, pinned to 2.6.13 and its build image by digest
  (`tacplus.lock`).
- **What works:** an empty tenant-owned tacplus Rust service, `skipper` (registry id 110): the proto package
  `skipper` with no rpcs, the binary on the generated `ServiceManager` with an empty handler, and `just gates`
  (tacplus build, `cargo build --locked`, `cargo test --locked`) passing.
- **Gaps:** no rpcs yet (the ticket API, the stream to the Admiral); no release image or workflow, and no CI
  workflow running `just gates` (PROPOSALS.md). `deny.toml` clarifies the generated crates' license
  until tacplus writes it (tacplus-dev/tacplus#368).
- **Blockers:** none.
