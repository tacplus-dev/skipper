# History: skipper

Append-only: add entries at the end, never edit or remove one (CI enforces it). One line each:
`- <YYYY-MM-DD> <who: looper|agent|jody> <kind: issue|decision|release|deploy> <one line> (<links>)`

- 2026-11-19 jody decision Created, from the Skipper/Admiral huddles (tacplus-dev/roadmap decisions/skipper-and-admiral-2026-11-01.md)
- 2026-11-20 looper issue Scaffold Skipper as a tacplus Rust service: tacplus.yaml, its proto package, tacplus.lock pin, just gates (#1)
- 2026-11-20 looper issue Ticket API: Report, Status, Updates for the tenant's authenticated users and agents (#2)
- 2026-11-20 looper issue The LLM advisor: read-only tools plus propose, and the build gate that keeps it from any token or write path (#5)
