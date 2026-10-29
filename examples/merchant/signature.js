'use strict';
const crypto = require('node:crypto');

const TOLERANCE_SECS = 300;

// Safrod-Signature: t=<unix seconds>,v1=<hex HMAC-SHA256 of "t.body">
function verify(secret, header, rawBody, nowSecs = Math.floor(Date.now() / 1000)) {
  if (typeof header !== 'string') return { ok: false, reason: 'missing' };
  let t = null;
  const candidates = [];
  for (const part of header.split(',')) {
    const [k, v] = part.trim().split('=');
    if (k === 't') t = Number(v);
    if (k === 'v1' && /^[0-9a-f]+$/i.test(v || '')) candidates.push(Buffer.from(v, 'hex'));
  }
  if (!Number.isInteger(t) || candidates.length === 0) return { ok: false, reason: 'malformed' };
  const expected = crypto.createHmac('sha256', secret).update(`${t}.`).update(rawBody).digest();
  // The MAC is checked before the clock so a forger learns nothing from the error type.
  const match = candidates.some((c) => c.length === expected.length && crypto.timingSafeEqual(c, expected));
  if (!match) return { ok: false, reason: 'mismatch' };
  if (Math.abs(nowSecs - t) > TOLERANCE_SECS) return { ok: false, reason: 'stale' };
  return { ok: true };
}

function sign(secret, t, rawBody) {
  const mac = crypto.createHmac('sha256', secret).update(`${t}.`).update(rawBody).digest('hex');
  return `t=${t},v1=${mac}`;
}

module.exports = { verify, sign, TOLERANCE_SECS };
