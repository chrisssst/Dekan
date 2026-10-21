import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  MAX_CIPHERTEXT_CHARS,
  MAX_MEMBERS,
  MAX_MESSAGE_BYTES,
  MAX_NONCE_CHARS,
  isMemberId,
  isRoomFull,
  isRoomKey,
  isSealedBlob,
  parseMessage,
  readSkin,
  reasonOf,
} from '../src/protocol.ts';

const blob = { v: 1, n: 'abc', c: 'def' };

describe('limits', () => {
  it('match the Rust relay', () => {
    assert.equal(MAX_MEMBERS, 5);
    assert.equal(MAX_MESSAGE_BYTES, 8192);
  });
});

describe('isRoomFull', () => {
  it('accepts a join below the member limit', () => {
    for (let open = 0; open < MAX_MEMBERS; open += 1) assert.equal(isRoomFull(open), false);
  });

  it('refuses the sixth member', () => {
    assert.equal(isRoomFull(5), true);
    assert.equal(isRoomFull(6), true);
  });
});

describe('isRoomKey', () => {
  it('accepts 32 lowercase hex characters', () => {
    assert.equal(isRoomKey('0123456789abcdef0123456789abcdef'), true);
  });

  it('rejects other lengths, case and characters', () => {
    assert.equal(isRoomKey(''), false);
    assert.equal(isRoomKey('0123456789abcdef0123456789abcde'), false);
    assert.equal(isRoomKey('0123456789abcdef0123456789abcdef0'), false);
    assert.equal(isRoomKey('0123456789ABCDEF0123456789ABCDEF'), false);
    assert.equal(isRoomKey('0123456789abcdef0123456789abcdeg'), false);
    assert.equal(isRoomKey('0123456789abcdef0123456789abcdef\n'), false);
  });
});

describe('isMemberId', () => {
  it('accepts positive safe integers', () => {
    assert.equal(isMemberId(1), true);
    assert.equal(isMemberId(Number.MAX_SAFE_INTEGER), true);
  });

  it('rejects everything else', () => {
    for (const value of [0, -1, 1.5, NaN, Infinity, 2 ** 53, '1', null, undefined, {}, [1], true]) {
      assert.equal(isMemberId(value), false, String(value));
    }
  });
});

describe('isSealedBlob', () => {
  it('accepts version 1 with strings inside the limits', () => {
    assert.equal(isSealedBlob(blob), true);
    assert.equal(isSealedBlob({ v: 1, n: 'n'.repeat(MAX_NONCE_CHARS), c: 'c'.repeat(MAX_CIPHERTEXT_CHARS) }), true);
    assert.equal(isSealedBlob({ v: 1, n: '', c: '' }), true);
  });

  it('rejects other versions and missing or mistyped fields', () => {
    assert.equal(isSealedBlob({ v: 2, n: 'a', c: 'b' }), false);
    assert.equal(isSealedBlob({ v: '1', n: 'a', c: 'b' }), false);
    assert.equal(isSealedBlob({ n: 'a', c: 'b' }), false);
    assert.equal(isSealedBlob({ v: 1, c: 'b' }), false);
    assert.equal(isSealedBlob({ v: 1, n: 'a' }), false);
    assert.equal(isSealedBlob({ v: 1, n: 1, c: 'b' }), false);
    assert.equal(isSealedBlob({ v: 1, n: 'a', c: 1 }), false);
  });

  it('rejects oversize nonce and ciphertext', () => {
    assert.equal(isSealedBlob({ v: 1, n: 'n'.repeat(MAX_NONCE_CHARS + 1), c: 'c' }), false);
    assert.equal(isSealedBlob({ v: 1, n: 'n', c: 'c'.repeat(MAX_CIPHERTEXT_CHARS + 1) }), false);
  });

  it('rejects non-objects', () => {
    for (const value of [null, undefined, 1, 'x', true]) assert.equal(isSealedBlob(value), false);
  });
});

describe('readSkin', () => {
  it('keeps null as a cleared skin', () => {
    assert.equal(readSkin(null), null);
  });

  it('returns a copy holding only v, n and c', () => {
    const input = { ...blob, extra: 'x' };
    const skin = readSkin(input);
    assert.deepEqual(skin, blob);
    assert.notEqual(skin, input);
  });

  it('returns undefined for an invalid blob or a missing skin', () => {
    assert.equal(readSkin(undefined), undefined);
    assert.equal(readSkin({ v: 2, n: 'a', c: 'b' }), undefined);
    assert.equal(readSkin('blob'), undefined);
  });
});

describe('parseMessage', () => {
  it('parses a JSON object', () => {
    assert.deepEqual(parseMessage('{"type":"join","summoner_id":7}'), {
      ok: true,
      msg: { type: 'join', summoner_id: 7 },
    });
  });

  it('closes with 1003 on a binary message', () => {
    assert.deepEqual(parseMessage(new ArrayBuffer(4)), {
      ok: false,
      reason: 'not_text',
      fields: {},
      close: { code: 1003, text: 'text only' },
    });
  });

  it('closes with 1009 above the byte limit', () => {
    const message = `{"a":"${'x'.repeat(MAX_MESSAGE_BYTES)}"}`;
    const result = parseMessage(message);
    assert.deepEqual(result, {
      ok: false,
      reason: 'too_large',
      fields: { bytes: message.length, max: MAX_MESSAGE_BYTES },
      close: { code: 1009, text: 'message too large' },
    });
  });

  it('measures bytes, not characters', () => {
    const message = 'é'.repeat(MAX_MESSAGE_BYTES / 2 + 1);
    assert.ok(message.length < MAX_MESSAGE_BYTES);
    const result = parseMessage(message);
    assert.equal(result.ok, false);
    if (!result.ok) assert.equal(result.reason, 'too_large');
  });

  it('accepts exactly the byte limit', () => {
    const padding = 'x'.repeat(MAX_MESSAGE_BYTES - '{"a":""}'.length);
    const message = `{"a":"${padding}"}`;
    assert.equal(new TextEncoder().encode(message).length, MAX_MESSAGE_BYTES);
    assert.equal(parseMessage(message).ok, true);
  });

  it('drops invalid JSON without closing', () => {
    const result = parseMessage('{not json');
    assert.equal(result.ok, false);
    if (result.ok) return;
    assert.equal(result.reason, 'invalid_json');
    assert.equal(typeof result.fields.error, 'string');
    assert.equal(result.close, undefined);
  });

  it('drops JSON that is not an object without closing', () => {
    for (const message of ['null', '[]', '[1,2]', '1', '"text"', 'true']) {
      assert.deepEqual(parseMessage(message), { ok: false, reason: 'not_an_object', fields: {} }, message);
    }
  });
});

describe('reasonOf', () => {
  it('uses the message of an Error and stringifies anything else', () => {
    assert.equal(reasonOf(new Error('boom')), 'boom');
    assert.equal(reasonOf('plain'), 'plain');
    assert.equal(reasonOf(42), '42');
  });
});
