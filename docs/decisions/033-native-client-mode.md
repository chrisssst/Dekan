# ADR-033: Opt-in native client mode

Status: accepted by the maintainer for Dekan 2.3.

## Context

The public upstream source omits its private client synchronizer. The maintainer requested the native League client menu and carousel shown in the supplied screenshot, using Dekan identity and destinations. This explicitly replaces the earlier standalone-only selection convention.

## Decision

Recover the embedded CSS and JavaScript from the supplied 1.16 installer without executing it. Original byte slice hashes and offsets are in the native client web recovery manifest; those hashes describe the original extraction, not the subsequently rebranded files. Replace all script-owned identifiers, visible branding, logo data and links with Dekan values. Turkish and English are the only menu languages. Keep the supplied native launcher and DLL unchanged and hash verified, including the required internal compatibility alias.

Implement the missing Rust coordination in the application selection segment. Reuse the existing workspace tokio-tungstenite dependency for CDP and base64 for embedding the original Dekan logo rather than adding a different transport or duplicating the image. These are additional consumers of already locked dependencies; no new runtime package is introduced. The transport accepts only a page WebSocket on the loopback port read from LeagueClientUx command-line arguments. HTTP redirects and proxies are disabled; messages, discovery bodies, queued commands and evaluation time are bounded. The CDP task is owned by the existing supervised overlay session and the blocking helper handle is retained and observed.

The verified helper installs and removes only its own IFEO debugger. Elevation is restricted to this helper; the application manifest remains asInvoker. A cancelled or failed helper attempt keeps the standalone UI and is not repeatedly prompted. Activation restart is allowed only while idle or in a lobby. No changes are made to game arming, injection timing, relay encryption, signing configuration or Discord Rich Presence.

On successful initialization, carousel commands flow through the existing catalog validation and state transitions. The standalone picker is hidden only when a matching champion catalog and actual client carousel are present. Client loss, reconnect, disabled mode or failed evaluation restores the picker. Cleanup removes the script, timers, event handlers and hooks and requests removal of the registered helper belonging to this launcher. A refused removal is logged and can be retried by disabling the mode on the next launch.

## Validation

Rust tests cover catalog shape, bounded typed commands, destinations, exact debug-port parsing, local endpoint restrictions and a mock HTTP/WebSocket exchange with CDP request correlation and exception handling. A Node DOM fixture checks recovered script syntax, menu creation, language, click commands and cleanup. It uses linkedom 0.18.12 only in a separately installed test directory; this package is not a build or runtime dependency. Run it with `DEKAN_DOM_MODULE=<absolute test directory>/node_modules/linkedom node tests/native-client/ui.mjs`. The installer records its selected language in language.ini; installed UI and the native menu prefer this Turkish/English selection over the League locale. Portable builds retain their existing locale behavior. Windows builds, UI tests and packaging run through the existing CI. A live client and match test is still required before promotion to latest.
