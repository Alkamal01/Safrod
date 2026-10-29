'use strict';
const test = require('node:test');
const assert = require('node:assert');
const { verify, sign } = require('./signature');
const { applyEvent, orders, seenEvents } = require('./server');

const SECRET = 'whsec_test';

test('accepts a valid signature', () => {
  const body = Buffer.from('{"a":1}');
  assert.deepStrictEqual(verify(SECRET, sign(SECRET, 1000, body), body, 1010), { ok: true });
});

test('rejects a tampered body, wrong secret, stale timestamp and malformed headers', () => {
  const body = Buffer.from('{"a":1}');
  const header = sign(SECRET, 1000, body);
  assert.strictEqual(verify(SECRET, header, Buffer.from('{"a":2}'), 1000).reason, 'mismatch');
  assert.strictEqual(verify('other', header, body, 1000).reason, 'mismatch');
  assert.strictEqual(verify(SECRET, header, body, 5000).reason, 'stale');
  assert.strictEqual(verify(SECRET, 't=1', body, 1).reason, 'malformed');
  assert.strictEqual(verify(SECRET, undefined, body, 1).reason, 'missing');
});

test('matches the fixed vector asserted by the Rust tests', () => {
  assert.strictEqual(sign(SECRET, 1000, Buffer.from('{"a":1}')), 't=1000,v1=1007d8ff5e2ecbd6b41db303d180ae70972ed8eb08209f2c93826a8b72775200');
});

test('a repeated webhook event is applied once', () => {
  orders.set('o1', { id: 'o1', invoice_id: 'inv_1', status: 'awaiting_payment' });
  const event = { id: 'evt_1', type: 'invoice.paid', created_at: 'now', data: { invoice_id: 'inv_1' } };
  assert.strictEqual(applyEvent(event), 'applied');
  assert.strictEqual(applyEvent(event), 'duplicate');
  assert.strictEqual(orders.get('o1').status, 'paid');
  seenEvents.clear();
});
