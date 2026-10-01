# dekan-party

Party mode: friends on the same team see each other's chosen skins. The players' machines never connect to
each other directly. They exchange encrypted messages through a relay, and the relay cannot read them.

## Where it sits in the flow

1. **Create a room.** The host generates an invite token that holds a random 32-byte key, the time it was
   issued and a random member id. The token is shared as text. It expires, so an old invite cannot be reused
   indefinitely.
2. **Join.** Every member derives two values from the key:
   - the **room id** (a SHA-256 of the key with a fixed label), which is the only thing the relay sees, and
   - the **encryption key** (a SHA-256 of the key with a different label).
   The relay never learns the key, so it cannot decrypt anything.
3. **Announce.** When you pick a skin, Dekan encrypts your champion and skin with XChaCha20-Poly1305 and sends
   the result to the room. On the wire, a member is identified by a random number that is new every session,
   never by an account id or summoner name.
4. **Receive and verify.** Teammates' announcements are decrypted and passed to the app. Only announcements
   whose champion matches the real team roster reported by the League client are used. The skins then go into
   the same overlay as yours.

## What is inside

| File | Purpose |
| --- | --- |
| `token.rs` | Invite token: generation, encoding, decoding and expiry; the key is never printed in logs |
| `crypto.rs` | Room id derivation and the room cipher |
| `protocol.rs` | Messages exchanged with the relay and the size limits of the encrypted payload |
| `client.rs` | Connection to the relay: join, announce, receive members, reconnect |
| `config.rs` | Which relay to use: `DEKAN_RELAY_URL`, then `party.json` in Dekan's state folder, then the built-in default |
| `error.rs` | Error type |

## Relays

- The public relay runs on Cloudflare Workers (`relay-worker/` at the repository root).
- `dekan-relay` is a Rust server that speaks the same protocol, for anyone who wants to host their own.

Rooms hold at most five members, one full team. A sixth player is turned away with a "room full" answer, and
Dekan tells the user without retrying.

## Testing

```powershell
cargo test -p dekan-party
```

The integration tests start a local relay and run several clients against it.
