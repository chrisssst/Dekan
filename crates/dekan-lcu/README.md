# dekan-lcu

Talks to the League client. The client exposes a local API, the LCU, on `127.0.0.1`: a REST interface and a
WebSocket event stream. This crate is Dekan's only way of knowing what is happening in the client.

## Where it sits in the flow

1. **Connect.** When the client starts, it writes a lockfile with the port and a password for its local API.
   Dekan reads it and connects. The client uses a self-signed certificate, which is accepted only for
   `127.0.0.1`.
2. **Observe.** Dekan subscribes to the gameflow and champion select events over the WebSocket and turns them
   into game phases and team rosters in the shared app state. If the connection drops, it reconnects with an
   increasing delay plus some randomness, so it never hammers a client that is restarting.
3. **Re-read before acting.** Just before the overlay is built, the current champion and selection are fetched
   again over REST instead of trusted from earlier events. This catches late changes such as an ARAM bench
   swap, a trade, or a lock in the final second.
4. **Register the skin.** The client is told which skin to show on your loading card. A skin you own is
   registered as itself. For a skin you do not own, the client only accepts the champion's default skin, so
   the card shows the default name while the skin itself still loads in game.

## What is inside

| File | Purpose |
| --- | --- |
| `lockfile.rs` | Finds and parses the client lockfile; builds the REST and WebSocket addresses |
| `client.rs` | REST client; every request has a timeout |
| `websocket.rs` | WebSocket connection and reconnection |
| `observer.rs` | Turns client events into game phases and app state updates |
| `champ_select.rs` | Champion select session: both teams, picks, selected skins, when the phase is really over |
| `live_selection.rs` | The final, authoritative read of champion and skin, used right before building the overlay; covers ARAM, Arena and rotating modes |
| `skin_registration.rs` | Which skin id to register with the client and how |
| `champion_assets.rs` | Skin and chroma lists from the client, used to fill the selection window when no local library exists |

## Design notes

- **The client is the source of truth** for your champion, the game phase and the skins you own. Dekan does
  not rely on stale copies of that data.
- **The skin to inject is not taken from the client.** The client cannot express a skin you do not own, so
  the skin to inject always comes from Dekan's own selection window.
- **Logs record changes, not polls.** A phase change is logged once, and a repeated "waiting for the client"
  message is never written in a loop.

## Testing

```powershell
cargo test -p dekan-lcu
```

The tests use recorded client payloads, so no running client is needed.
