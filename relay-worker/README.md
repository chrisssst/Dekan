# relay-worker

The public party relay, running on Cloudflare Workers. It connects the members of a party room and passes
their encrypted messages between them. It never sees account ids, champions or skins, only encrypted blobs.

## How it works

- Each room is a Durable Object, keyed by a 32-character hex room id that the clients derive from their party
  key. The key itself never reaches the relay.
- A room holds at most **5 members**, one team. A sixth connection gets `409 Room is full` before the upgrade, and Dekan tells
  the user without retrying. The Rust relay answers the same way, and closes with code 4009 when the room fills
  during the handshake.
- Each member can publish one encrypted blob. Whenever a blob changes, the room broadcasts the updated member
  list to everyone in it, at most once every 500 ms: changes inside that window are sent together by a Durable
  Object alarm, so a member flooding the room cannot multiply the outgoing traffic and the last state is never
  lost. Members leaving are broadcast at once.
- A member id that joins again (a reconnect after the network dropped) replaces its older connection, which is
  closed with code 4000, so the room never lists the same member twice.
- Messages are limited to 8 KiB, and a blob's nonce and ciphertext have their own size limits. Anything larger
  or malformed is dropped and logged.

It speaks the same protocol as the Rust relay in `crates/dekan-relay`, so either one can be used.

## Deploying

```powershell
npm ci
npm run check          # strict TypeScript typecheck
npx wrangler login
npm run deploy
```

The deploy prints the worker address (`https://dekan-party-relay.<account>.workers.dev`). Point Dekan at it
with `wss://`, either with the environment variable
`DEKAN_RELAY_URL=wss://dekan-party-relay.<account>.workers.dev` or in
`%LOCALAPPDATA%\Dekan\state\party.json`:

```json
{ "relay_url": "wss://dekan-party-relay.<account>.workers.dev" }
```

## Local development

```powershell
npm run dev            # ws://127.0.0.1:8787
```

Dekan only accepts plain `ws://` for loopback addresses. Every other relay has to use `wss://`.

## Logs

`wrangler.toml` enables Workers Logs. Every line is a JSON object with an `event` field, such as
`join_refused_room_full`, `member_replaced` or `message_dropped` (with a `reason`). Rooms, members and blobs are
never logged, only counts, sizes and reasons.

## Tests

```powershell
npm test
```

The tests use Node's built-in runner (`node --test`, Node 24 or newer) and run the TypeScript files directly,
so they add no dependency. They cover `src/protocol.ts`, where the protocol decisions live: message parsing and
size limits, member id validation, the room-full limit, room keys and skin blobs. `src/room.ts` only applies
those decisions to the Durable Object, which needs the Workers runtime.

CI runs `npm ci`, `npm audit`, `npm run check` and `npm test` on every push.
