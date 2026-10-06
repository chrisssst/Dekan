# dekan-relay

A self-hosted party relay written in Rust. It speaks the same protocol as the public relay in `relay-worker/`,
so anyone who does not want to rely on the public relay can run their own.

## What it does

It keeps a set of rooms in memory. A room is identified by a 32-character hex id derived from the party key; the
key itself never reaches the relay. Each member of a room can publish one encrypted blob, and every change is
broadcast to the whole room. The relay cannot read the blobs. It only checks their shape and size and passes
them along.

Limits:

- at most **5 members** per room (one team); a sixth connection gets `409 Room is full`,
- at most **8 KiB** per message,
- a room disappears when its last member leaves.

Endpoints:

- `GET /room?key=<room id>`: WebSocket upgrade into a room,
- anything else answers with a small JSON health response, which is handy for uptime checks.

## Running it

```powershell
cargo run -p dekan-relay --release
```

| Variable | Default | Meaning |
| --- | --- | --- |
| `HOST` | `0.0.0.0` | Address to listen on |
| `PORT` | `8787` | Port to listen on |
| `RUST_LOG` | `info` | Log level |

Put it behind a TLS terminator (a reverse proxy or tunnel) and point Dekan at it with `wss://`, either through
`DEKAN_RELAY_URL` or through `party.json` in `%LOCALAPPDATA%\Dekan\state`. Plain `ws://` is only meant for
local testing.

`Ctrl+C` stops the server cleanly.

## Testing

```powershell
cargo test -p dekan-relay
```
