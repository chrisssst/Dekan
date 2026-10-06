import { DurableObject } from 'cloudflare:workers';

interface SealedBlob {
  v: number;
  n: string;
  c: string;
}

interface MemberInfo {
  summoner_id: number;
  summoner_name: string;
  skin?: SealedBlob | null;
}

const MAX_MEMBERS = 5;
const MAX_MESSAGE_BYTES = 8 * 1024;
const MAX_NONCE_CHARS = 64;
const MAX_CIPHERTEXT_CHARS = 4096;

function isSealedBlob(value: unknown): value is SealedBlob {
  if (typeof value !== 'object' || value === null) return false;
  const blob = value as Record<string, unknown>;
  return (
    blob.v === 1 &&
    typeof blob.n === 'string' &&
    blob.n.length <= MAX_NONCE_CHARS &&
    typeof blob.c === 'string' &&
    blob.c.length <= MAX_CIPHERTEXT_CHARS
  );
}

type RoomEnv = Record<string, never>;

export class PartyRoom extends DurableObject<RoomEnv> {
  constructor(ctx: DurableObjectState, env: RoomEnv) {
    super(ctx, env);
    this.ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair('ping', 'pong'));
  }

  async fetch(_request: Request): Promise<Response> {
    const open = this.ctx
      .getWebSockets()
      .filter((ws) => ws.readyState === WebSocket.READY_STATE_OPEN);
    if (open.length >= MAX_MEMBERS) {
      return new Response('Room is full', { status: 409 });
    }

    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    this.ctx.acceptWebSocket(server);
    server.send(JSON.stringify({ type: 'members', members: this.getMembers() }));
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer) {
    if (typeof message !== 'string' || message.length > MAX_MESSAGE_BYTES) return;

    let msg: Record<string, unknown>;
    try {
      msg = JSON.parse(message);
    } catch {
      return;
    }

    switch (msg.type) {
      case 'join': {
        const id = msg.summoner_id;
        if (typeof id !== 'number' || !Number.isSafeInteger(id) || id <= 0) return;
        const info: MemberInfo = { summoner_id: id, summoner_name: '' };
        ws.serializeAttachment(info);
        this.broadcastMembers();
        break;
      }
      case 'skin': {
        const existing = ws.deserializeAttachment() as MemberInfo | null;
        if (!existing) return;
        if (msg.skin === null) {
          existing.skin = null;
        } else if (isSealedBlob(msg.skin)) {
          existing.skin = { v: msg.skin.v, n: msg.skin.n, c: msg.skin.c };
        } else {
          return;
        }
        ws.serializeAttachment(existing);
        this.broadcastMembers();
        break;
      }
      case 'leave': {
        ws.close(1000, 'client left');
        break;
      }
    }
  }

  async webSocketClose(ws: WebSocket) {
    ws.serializeAttachment(null);
    this.broadcastMembers();
  }

  async webSocketError(ws: WebSocket) {
    ws.serializeAttachment(null);
    this.broadcastMembers();
  }

  private getMembers(): MemberInfo[] {
    const members: MemberInfo[] = [];
    for (const ws of this.ctx.getWebSockets()) {
      if (ws.readyState !== WebSocket.READY_STATE_OPEN) continue;
      const info = ws.deserializeAttachment() as MemberInfo | null;
      if (info?.summoner_id) members.push(info);
    }
    return members;
  }

  private broadcastMembers() {
    const payload = JSON.stringify({ type: 'members', members: this.getMembers() });
    for (const ws of this.ctx.getWebSockets()) {
      if (ws.readyState !== WebSocket.READY_STATE_OPEN) continue;
      try {
        ws.send(payload);
      } catch {
      }
    }
  }
}
