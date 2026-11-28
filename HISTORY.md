# History: skipper

Append-only: add entries at the end, never edit or remove one (CI enforces it). One line each:
`- <YYYY-MM-DD> <who: looper|agent|jody> <kind: issue|decision|release|deploy> <one line> (<links>)`

- 2026-11-19 jody decision Created, from the Skipper/Admiral huddles (tacplus-dev/roadmap decisions/skipper-and-admiral-2026-11-01.md)
- 2026-11-20 looper issue Scaffold Skipper as a tacplus Rust service: tacplus.yaml, its proto package, tacplus.lock pin, just gates (#1)
- 2026-11-20 looper issue Ticket API: Report, Status, Updates for the tenant's authenticated users and agents (#2)
- 2026-11-20 looper issue The LLM advisor: read-only tools plus propose, and the build gate that keeps it from any token or write path (#5)
- 2026-11-23 looper issue Pinned images behind their lines: relock (tacplus.lock: tacplus 2.6.13 → 2.6.14, and 1 more) (#7)
- 2026-11-23 looper issue Dependencies: url 2.5.8 (RUSTSEC-2026-0592, stack overflow in Url::origin on nested blob: URLs): update to 2.5.9, lockfile-only (#8)
- 2026-11-28 looper issue Pinned images behind their lines: relock (tacplus.lock: tacplus 2.6.14 → 2.6.16, and 1 more) (#9)
