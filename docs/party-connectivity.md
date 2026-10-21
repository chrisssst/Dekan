# Party connectivity and automatic DNS recovery

## Observed failure

On 2026-10-21 a Windows client resolved the default relay to Cloudflare addresses that refused TCP port 443.
The application logged Windows error 10061 before a WebSocket connection was established. The same machine
could reach another Cloudflare address. An HTTPS request to that address, retaining the Dekan hostname and
normal certificate verification, returned HTTP 200 and the `dekan-party-relay` service identity. This identifies
an address-dependent reachability failure; it does not identify which resolver, ISP or network device caused it.

## Automatic recovery

The application first attempts its normal system-resolved WebSocket connection. For the exact default relay
hostname on secure port 443, an I/O error or ten-second connection timeout enables a second path:

1. Query Google Public DNS and Cloudflare DNS over HTTPS concurrently, with three-second request deadlines.
   Queries contain only the public relay hostname and request A records. Google queries suppress client subnet
   disclosure with `edns_client_subnet=0.0.0.0/0`. No room id, invite token, account id or encrypted pick is included.
2. Read at most 16 KiB per response. Reject malformed, unsuccessful, truncated or checking-disabled answers;
   accept only matching-name public IPv4 A records, deduplicated and capped at four addresses.
3. Try the returned addresses with four-second TCP/TLS/WebSocket deadlines per address. The original URL is
   passed unchanged to the TLS and WebSocket library, retaining SNI, certificate hostname verification, Host
   and the room path. Certificate, HTTP and protocol failures stop address failover; a full room still yields 409.

The enclosing party client limits the entire attempt to 30 seconds and cancellation drops the connection and
DNS futures. Existing exponential reconnect backoff remains in effect. Each recovery resolves fresh addresses;
no fixed Cloudflare IP is shipped or persisted. Normal successful connections never contact the HTTPS DNS
services. Custom relays and loopback relays retain system resolution and are never disclosed to these services.

This cannot guarantee access on every network. If the DNS services or all returned relay addresses are
unreachable, the connection remains failed and the normal retry loop reports that failure.

## Verification

Deterministic tests cover DNS response validation, address limits, private/special-address rejection, recovery
scope, a refused address followed by a working local WebSocket with the original Host and room path, preservation
of room-full refusal, and cancellation of an incomplete handshake.

The live test forces the recovery path after a synthetic connection refusal, opens two certificate-checked
WebSocket connections to a random room on the deployed relay, waits for membership, sends an encrypted pick and
verifies decryption at the second client:

```powershell
cargo test -p dekan-party live_relay_recovers_from_refusal_and_relays_an_encrypted_pick -- --ignored --nocapture
```

The Windows quality gate also runs this live probe. It is ignored by default on other platforms, where the
command above opts in. A release still needs a Windows test on the affected network, with any temporary hosts
entry removed, and a real match log before stable promotion.

## References

- [Google HTTPS DNS JSON API](https://developers.google.com/speed/public-dns/docs/doh/json)
- [Cloudflare HTTPS DNS JSON API](https://developers.cloudflare.com/1.1.1.1/encryption/dns-over-https/make-api-requests/dns-json/)
- [TLS WebSocket connection over an existing stream](https://docs.rs/tokio-tungstenite/0.30.0/tokio_tungstenite/fn.client_async_tls_with_config.html)
