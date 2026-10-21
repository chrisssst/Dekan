export interface SealedBlob {
  v: number;
  n: string;
  c: string;
}

export const MAX_MEMBERS = 5;
export const MAX_MESSAGE_BYTES = 8 * 1024;
export const MAX_NONCE_CHARS = 64;
export const MAX_CIPHERTEXT_CHARS = 4096;

const ROOM_KEY = /^[0-9a-f]{32}$/;
const encoder = new TextEncoder();

export type Parsed =
  | { ok: true; msg: Record<string, unknown> }
  | { ok: false; reason: string; fields: Record<string, unknown>; close?: { code: number; text: string } };

export function reasonOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function isRoomKey(value: string): boolean {
  return ROOM_KEY.test(value);
}

export function isRoomFull(open: number): boolean {
  return open >= MAX_MEMBERS;
}

export function isMemberId(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0;
}

export function isSealedBlob(value: unknown): value is SealedBlob {
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

export function readSkin(value: unknown): SealedBlob | null | undefined {
  if (value === null) return null;
  if (!isSealedBlob(value)) return undefined;
  return { v: value.v, n: value.n, c: value.c };
}

export function parseMessage(message: string | ArrayBuffer): Parsed {
  if (typeof message !== 'string') {
    return { ok: false, reason: 'not_text', fields: {}, close: { code: 1003, text: 'text only' } };
  }
  const bytes = encoder.encode(message).length;
  if (bytes > MAX_MESSAGE_BYTES) {
    return {
      ok: false,
      reason: 'too_large',
      fields: { bytes, max: MAX_MESSAGE_BYTES },
      close: { code: 1009, text: 'message too large' },
    };
  }
  try {
    const parsed: unknown = JSON.parse(message);
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
      return { ok: false, reason: 'not_an_object', fields: {} };
    }
    return { ok: true, msg: parsed as Record<string, unknown> };
  } catch (error) {
    return { ok: false, reason: 'invalid_json', fields: { error: reasonOf(error) } };
  }
}
