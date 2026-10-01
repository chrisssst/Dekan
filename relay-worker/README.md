# relay-worker

The public party relay, running on Cloudflare Workers. It connects the members of a party room and passes
their encrypted messages between them. It never sees account ids, champions or skins, only encrypted blobs.

## How it works

- Each room is a Durable Object, keyed by a 32-character hex room id that the clients derive from their party
  key. The key itself never reaches the relay.
- A room holds at most **5 members**, one team. A sixth connection gets `409 Room is full`, and Dekan tells
  the user without retrying.
- Each member can publish one encrypted blob. Whenever a blob changes, the room broadcasts the updated member
  list to everyone in it.
- Messages are limited to 8 KiB, and a blob's nonce and ciphertext have their own size limits. Anything larger
  or malformed is rejected.

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

CI runs `npm run check` on every push.
