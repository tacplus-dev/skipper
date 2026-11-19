# Roadmap: skipper

Owned by Jody; changed only by an agent at Jody's request. Loopers propose in PROPOSALS.md.
Design: tacplus-dev/roadmap `decisions/skipper-and-admiral-2026-11-01.md`.

## Now
- Scaffold Skipper as a tacplus Rust service, tenant-scope, pinned in tacplus.lock, gates green (#1). Hub, at Jody's request, 2026-11-19.
- Ticket API: Report, Status, Updates for the tenant's authenticated users and agents, rate-limited, no anonymous reports (#2). Hub, at Jody's request, 2026-11-19.
- The dial-out bidirectional stream to the Admiral, typed both ways (#3), on the API of tacplus-dev/admiral#3. Hub, at Jody's request, 2026-11-19.
- Validate a report: reproduce, sanitized typed evidence, classify; unreproduced reports still sent (#4). Hub, at Jody's request, 2026-11-19.
- The LLM advisor (read-only tools plus propose) and the build gate that keeps it from any token or write path (#5). Hub, at Jody's request, 2026-11-19.

## Next
- Assisted tenant configuration: redacted read, typed ConfigChange, user approval, per-field write with the user's on-behalf-of token (#6), after tacplus-dev/kernel#35. Hub, at Jody's request, 2026-11-19.

## Later
- A chat front for reporters.

## Not doing
- Any write or token on the LLM's side (the platform rule).
- Filing into a tenant's own ticketing system (the Admiral files; tenants need none).
