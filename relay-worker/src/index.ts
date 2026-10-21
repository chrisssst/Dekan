import { isRoomKey } from './protocol.ts';

export { PartyRoom } from './room.ts';

interface Env {
  ROOM: DurableObjectNamespace;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);

    if (url.pathname === '/' && request.method === 'GET') {
      return new Response(JSON.stringify({ status: 'ok', service: 'dekan-party-relay' }), {
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (url.pathname === '/room' && request.headers.get('Upgrade')?.toLowerCase() === 'websocket') {
      const roomKey = url.searchParams.get('key') ?? '';
      if (!isRoomKey(roomKey)) {
        console.warn(JSON.stringify({ event: 'join_refused_invalid_key' }));
        return new Response('Invalid room key', { status: 400 });
      }
      try {
        const stub = env.ROOM.get(env.ROOM.idFromName(roomKey));
        return await stub.fetch(request);
      } catch (error) {
        console.error(
          JSON.stringify({ event: 'room_unavailable', error: error instanceof Error ? error.message : String(error) }),
        );
        return new Response('Room unavailable', { status: 503 });
      }
    }

    console.warn(JSON.stringify({ event: 'request_refused', status: 426 }));
    return new Response('Dekan party relay - WebSocket upgrade required at /room', { status: 426 });
  },
};
