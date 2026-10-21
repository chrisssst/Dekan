import { DurableObject } from 'cloudflare:workers';

import { MAX_MEMBERS, isMemberId, isRoomFull, parseMessage, readSkin, reasonOf } from './protocol.ts';
import type { SealedBlob } from './protocol.ts';

interface MemberInfo {
  summoner_id: number;
  summoner_name: string;
  skin?: SealedBlob | null;
}

const BROADCAST_INTERVAL_MS = 500;

function log(level: 'info' | 'warn' | 'error', event: string, fields: Record<string, unknown> = {}) {
  console[level](JSON.stringify({ event, ...fields }));
}

type RoomEnv = Record<string, never>;

export class PartyRoom extends DurableObject<RoomEnv> {
  private lastBroadcast = 0;

  constructor(ctx: DurableObjectState, env: RoomEnv) {
    super(ctx, env);
    this.ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair('ping', 'pong'));
  }

  async fetch(_request: Request): Promise<Response> {
    const open = this.ctx
      .getWebSockets()
      .filter((ws) => ws.readyState === WebSocket.READY_STATE_OPEN);
    if (isRoomFull(open.length)) {
      log('warn', 'join_refused_room_full', { open: open.length, max: MAX_MEMBERS });
      return new Response('Room is full', { status: 409 });
    }

    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    this.ctx.acceptWebSocket(server);
    server.send(JSON.stringify({ type: 'members', members: this.getMembers() }));
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer) {
    const parsed = parseMessage(message);
    if (!parsed.ok) {
      log('warn', 'message_dropped', { reason: parsed.reason, ...parsed.fields });
      if (parsed.close) ws.close(parsed.close.code, parsed.close.text);
      return;
    }
    const msg = parsed.msg;

    switch (msg.type) {
      case 'join': {
        const id = msg.summoner_id;
        if (!isMemberId(id)) {
          log('warn', 'join_dropped', { reason: 'invalid_member_id' });
          return;
        }
        const info: MemberInfo = { summoner_id: id, summoner_name: '' };
        this.dropStaleConnections(ws, id);
        ws.serializeAttachment(info);
        await this.scheduleBroadcast();
        break;
      }
      case 'skin': {
        const existing = ws.deserializeAttachment() as MemberInfo | null;
        if (!existing) {
          log('warn', 'skin_dropped', { reason: 'skin_before_join' });
          return;
        }
        const skin = readSkin(msg.skin);
        if (skin === undefined) {
          log('warn', 'skin_dropped', { reason: 'invalid_blob' });
          return;
        }
        existing.skin = skin;
        ws.serializeAttachment(existing);
        await this.scheduleBroadcast();
        break;
      }
      case 'leave': {
        ws.close(1000, 'client left');
        break;
      }
      default:
        log('warn', 'message_dropped', { reason: 'unknown_type' });
    }
  }

  async webSocketClose(ws: WebSocket) {
    this.forget(ws);
    this.broadcastMembers();
  }

  async webSocketError(ws: WebSocket, error: unknown) {
    log('warn', 'socket_error', { error: reasonOf(error) });
    this.forget(ws);
    this.broadcastMembers();
  }

  private forget(ws: WebSocket) {
    try {
      ws.serializeAttachment(null);
    } catch (error) {
      log('warn', 'forget_failed', { error: reasonOf(error) });
    }
  }

  private dropStaleConnections(current: WebSocket, id: number) {
    for (const ws of this.ctx.getWebSockets()) {
      if (ws === current) continue;
      const info = ws.deserializeAttachment() as MemberInfo | null;
      if (info?.summoner_id !== id) continue;
      this.forget(ws);
      log('info', 'member_replaced');
      try {
        ws.close(4000, 'replaced by a newer connection');
      } catch (error) {
        log('warn', 'replace_close_failed', { error: reasonOf(error) });
      }
    }
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

  async alarm() {
    this.broadcastMembers();
  }

  private async scheduleBroadcast() {
    const due = this.lastBroadcast + BROADCAST_INTERVAL_MS;
    if (Date.now() >= due) {
      this.broadcastMembers();
      return;
    }
    if ((await this.ctx.storage.getAlarm()) === null) {
      await this.ctx.storage.setAlarm(due);
    }
  }

  private broadcastMembers() {
    this.lastBroadcast = Date.now();
    const payload = JSON.stringify({ type: 'members', members: this.getMembers() });
    let failed = 0;
    let firstError: string | undefined;
    for (const ws of this.ctx.getWebSockets()) {
      if (ws.readyState !== WebSocket.READY_STATE_OPEN) continue;
      try {
        ws.send(payload);
      } catch (error) {
        failed += 1;
        firstError ??= reasonOf(error);
        this.forget(ws);
        try {
          ws.close(1011, 'send failed');
        } catch (closeError) {
          firstError ??= reasonOf(closeError);
        }
      }
    }
    if (failed > 0) log('warn', 'broadcast_send_failed', { failed, firstError });
  }
}
