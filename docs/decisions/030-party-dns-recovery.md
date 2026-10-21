# ADR-030: Recover default relay connections through HTTPS DNS

Status: accepted for implementation; affected-network release verification pending.

## Context

The default workers.dev relay is reachable from an affected Windows client through one Cloudflare edge address,
while the addresses returned by its system resolver refuse TCP connections. Users must not edit hosts files or
change system DNS to use party rooms. The project has no separately registered domain.

## Decision

Keep system resolution first and add a bounded HTTPS DNS recovery path scoped to the exact default relay.
Retain the original hostname for TLS certificate verification, SNI and WebSocket routing. Use dynamically
resolved A records rather than a baked-in Cloudflare edge IP. Do not apply public DNS recovery to custom relays,
HTTP refusals or certificate failures. DNS request and parsing limits are documented in
[`party-connectivity.md`](../party-connectivity.md).

Reuse the workspace's existing reqwest 0.12 dependency with its existing rustls and JSON features in dekan-party.
This adds no new third-party package or TLS implementation to the dependency graph. It avoids a new DNS client
dependency or a hand-written DNS wire parser. Both HTTPS DNS endpoints use the JSON API supported by the existing
serde stack.

## Consequences

Users with affected system-resolved addresses can recover without system changes if HTTPS DNS returns an
accessible address. Two existing public resolver operators learn the public relay hostname on recovery, but no
room or user identifiers. Recovery adds up to 30 seconds to a failed connection attempt, including the initial
attempt; cancellation and existing retry backoff are preserved. HTTPS DNS service availability becomes an
optional recovery dependency, with no guarantee on fully restricted networks.

The existing Windows quality gate runs the live encrypted relay probe; non-Windows runs require explicit opt-in.
This makes party CI depend on public resolver and relay availability. Proof also requires a Windows test on the
affected network. A green unit test suite alone does not establish end-user connectivity.
