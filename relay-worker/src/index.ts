export { PartyRoom } from './room';

interface Env {
  ROOM: DurableObjectNamespace;
}

const ROOM_KEY = /^[0-9a-f]{32}$/;

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
      if (!ROOM_KEY.test(roomKey)) {
        return new Response('Invalid room key', { status: 400 });
      }
      const stub = env.ROOM.get(env.ROOM.idFromName(roomKey));
      return stub.fetch(request);
    }

    return new Response('Dekan party relay - WebSocket upgrade required at /room', { status: 426 });
  },
};
