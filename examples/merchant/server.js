'use strict';
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { verify } = require('./signature');

// Connection details are re-read from this file so the setup script can
// provision keys without restarting the app.
const CONFIG_PATH = process.env.MERCHANT_CONFIG || path.join(__dirname, '..', '..', '.demo', 'merchant.json');
const PORT = Number(process.env.PORT || 4000);
const HOST = process.env.HOST || '127.0.0.1';

const orders = new Map();
const seenEvents = new Set();
const webhookLog = [];

function config() {
  try {
    return JSON.parse(fs.readFileSync(CONFIG_PATH, 'utf8'));
  } catch {
    return null;
  }
}

async function safrod(method, route, body, idempotencyKey) {
  const cfg = config();
  if (!cfg) throw new Error(`no merchant config at ${CONFIG_PATH}; run scripts/setup.sh`);
  const headers = { authorization: `Bearer ${cfg.api_key}`, 'content-type': 'application/json' };
  if (idempotencyKey) headers['idempotency-key'] = idempotencyKey;
  const res = await fetch(`${cfg.api_url}${route}`, { method, headers, body: body ? JSON.stringify(body) : undefined });
  const json = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(`${method} ${route} -> ${res.status} ${json.error ? json.error.message : ''}`);
  return json;
}

function readBody(req, limit = 64 * 1024) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    req.on('data', (c) => {
      size += c.length;
      if (size > limit) {
        reject(new Error('body too large'));
        req.destroy();
      } else chunks.push(c);
    });
    req.on('end', () => resolve(Buffer.concat(chunks)));
    req.on('error', reject);
  });
}

function send(res, status, body, type = 'application/json') {
  res.writeHead(status, { 'content-type': type, 'cache-control': 'no-store' });
  res.end(type === 'application/json' ? JSON.stringify(body) : body);
}

// Marks an order from a verified webhook. Safrod delivers at least once, so
// the same event can arrive twice; the event id makes handling idempotent.
function applyEvent(event) {
  if (seenEvents.has(event.id)) return 'duplicate';
  seenEvents.add(event.id);
  const data = event.data || {};
  for (const order of orders.values()) {
    if (event.type === 'invoice.paid' && order.invoice_id === data.invoice_id) {
      order.status = 'paid';
      order.paid_at = event.created_at;
    }
    if (event.type.startsWith('bitcoin.transaction.') && order.address === data.address) {
      order.confirmations = data.confirmations;
      order.txid = data.txid;
      order.status = data.status === 'confirmed' ? 'paid' : 'confirming';
    }
  }
  return 'applied';
}

async function handleWebhook(req, res) {
  const raw = await readBody(req);
  const cfg = config();
  if (!cfg || !cfg.webhook_secret) return send(res, 503, { error: 'webhook secret not configured' });
  const result = verify(cfg.webhook_secret, req.headers['safrod-signature'], raw);
  if (!result.ok) {
    webhookLog.push({ at: new Date().toISOString(), rejected: result.reason });
    return send(res, 401, { error: 'invalid signature' });
  }
  const event = JSON.parse(raw.toString('utf8'));
  const outcome = applyEvent(event);
  webhookLog.push({ at: new Date().toISOString(), id: event.id, type: event.type, simulated: event.simulated, outcome });
  console.log(`webhook ${event.type} ${event.id} ${outcome}`);
  return send(res, 200, { received: true });
}

async function createOrder(req, res) {
  const input = JSON.parse((await readBody(req)).toString('utf8') || '{}');
  const id = `order_${crypto.randomBytes(6).toString('hex')}`;
  if (input.kind === 'onchain') {
    const address = await safrod('POST', '/v1/bitcoin/addresses', { label: id, required_confirmations: 3 });
    const order = { id, kind: 'onchain', status: 'awaiting_payment', address: address.address, confirmations: 0 };
    orders.set(id, order);
    return send(res, 201, order);
  }
  const sats = Number(input.amount_sat || 1000);
  if (!Number.isInteger(sats) || sats < 1) return send(res, 422, { error: 'amount_sat must be a positive integer' });
  const invoice = await safrod('POST', '/v1/lightning/invoices', { amount_sat: sats, description: `Order ${id}`, metadata: { order_id: id } }, id);
  const order = { id, kind: 'lightning', status: 'awaiting_payment', invoice_id: invoice.id, bolt11: invoice.bolt11, amount_sat: sats };
  orders.set(id, order);
  return send(res, 201, order);
}

const PAGE = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Safrod example merchant</title>
<style>
 body{font:16px/1.5 system-ui,sans-serif;margin:0;background:#f6f7f9;color:#1b1f24}
 main{max-width:720px;margin:0 auto;padding:24px 16px}
 h1{font-size:1.3rem} .card{background:#fff;border:1px solid #d8dde3;border-radius:8px;padding:16px;margin:16px 0}
 button{font:inherit;padding:8px 14px;border-radius:6px;border:1px solid #2b5ea7;background:#2b5ea7;color:#fff;cursor:pointer}
 code{word-break:break-all;font-size:.8rem;display:block;background:#eef0f3;padding:8px;border-radius:6px;margin:8px 0}
 .paid{color:#15703a;font-weight:600} .note{color:#5b6570;font-size:.9rem}
</style></head><body><main>
<h1>Safrod example merchant <span class="note">(regtest, no real funds)</span></h1>
<p class="note">Orders are marked paid only when a signed webhook from Safrod arrives, not by polling.</p>
<div class="card"><button id="ln">Create a Lightning order (2,500 sats)</button> <button id="oc">Create an on-chain order</button></div>
<div id="orders"></div>
<script>
const ids=[];
async function create(body){const r=await fetch('/api/orders',{method:'POST',body:JSON.stringify(body)});const o=await r.json();ids.unshift(o.id);render()}
document.getElementById('ln').onclick=()=>create({kind:'lightning',amount_sat:2500});
document.getElementById('oc').onclick=()=>create({kind:'onchain'});
async function render(){
 const out=[];
 for(const id of ids){const o=await (await fetch('/api/orders/'+id)).json();
  out.push('<div class="card"><b>'+o.id+'</b> &middot; '+o.kind+' &middot; <span class="'+(o.status==='paid'?'paid':'')+'">'+o.status+'</span>'
   +(o.bolt11?'<code>'+o.bolt11+'</code>':'')+(o.address?'<code>'+o.address+'</code>confirmations: '+(o.confirmations||0):'')+'</div>')}
 document.getElementById('orders').innerHTML=out.join('')}
setInterval(render,1500);
</script></main></body></html>`;

const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://localhost');
    if (req.method === 'GET' && url.pathname === '/') return send(res, 200, PAGE, 'text/html; charset=utf-8');
    if (req.method === 'GET' && url.pathname === '/healthz') return send(res, 200, { status: 'ok' });
    if (req.method === 'POST' && url.pathname === '/webhook') return await handleWebhook(req, res);
    if (req.method === 'POST' && url.pathname === '/api/orders') return await createOrder(req, res);
    if (req.method === 'GET' && url.pathname === '/api/webhooks') return send(res, 200, webhookLog);
    const m = url.pathname.match(/^\/api\/orders\/([\w-]+)$/);
    if (req.method === 'GET' && m) {
      const order = orders.get(m[1]);
      return order ? send(res, 200, order) : send(res, 404, { error: 'not found' });
    }
    return send(res, 404, { error: 'not found' });
  } catch (err) {
    console.error(err.message);
    return send(res, 500, { error: 'internal error' });
  }
});

if (require.main === module) {
  server.listen(PORT, HOST, () => console.log(`merchant app on http://${HOST}:${PORT}`));
}

module.exports = { server, applyEvent, orders, seenEvents };
