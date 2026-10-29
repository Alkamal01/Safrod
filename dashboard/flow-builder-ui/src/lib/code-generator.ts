export interface ExamplePerson {
  fullName: string;
  nationality: string;
}

export interface CurrencySelection {
  code: string;
  type: 'fiat' | 'crypto';
  name: string;
  accountType: string;
  accountLabel: string;
  countryCode?: string;
  network?: string;
  jitEligible: boolean;
  isInternal: boolean;
  examplePerson: ExamplePerson;
}

export interface ApiStep {
  step: number;
  title: string;
  description: string;
  method: string;
  endpoint: string;
  headers: Record<string, string>;
  body?: Record<string, unknown>;
  note?: string;
  isInstructional?: boolean;
}

const API = 'http://127.0.0.1:8080';
const headers = {
  Authorization: 'Bearer $SAFROD_API_KEY',
  'Content-Type': 'application/json',
};

function post(step: number, title: string, endpoint: string, body: Record<string, unknown>, note?: string): ApiStep {
  return { step, title, description: endpoint.replace(`${API}`, ''), method: 'POST', endpoint: `${API}${endpoint}`, headers, body, note };
}

export function generateSteps(source: CurrencySelection, destination: CurrencySelection): ApiStep[] {
  const compatibleRail = source.type === destination.type && (
    source.type === 'fiat'
      ? source.code !== destination.code
      : source.network === destination.network
  );
  if (!compatibleRail) {
    return [{
      step: 1,
      title: 'Choose a supported test flow',
      description: 'No matching Safrod recipe',
      method: '', endpoint: '', headers: {}, isInstructional: true,
      note: 'Choose Lightning regtest for both ends, Bitcoin on-chain regtest for both ends, or two different sandbox currencies. Safrod does not convert between fiat and Bitcoin.',
    }];
  }

  const destNetwork = destination.network ?? '';

  if (destNetwork.toLowerCase().includes('lightning')) {
    return [
      post(1, 'Create a regtest invoice', '/v1/lightning/invoices', {
        amount_sat: 2500,
        description: 'Safrod flow builder demo',
        metadata: { flow: 'lightning-demo' },
      }, 'This creates a real invoice on the local regtest Lightning node. Copy its bolt11 value into the payment request below.'),
      { ...post(2, 'Pay from the other node', '/v1/lightning/payments', {
        bolt11: '<bolt11 from step 1>',
        metadata: { flow: 'lightning-demo' },
      }, 'Run this request against the independent payer node at http://127.0.0.1:8081 and use the payer API key. Reuse the same Idempotency-Key when retrying.'), endpoint: 'http://127.0.0.1:8081/v1/lightning/payments' },
      { step: 3, title: 'Check settlement', description: '/v1/lightning/invoices/{id}', method: 'GET', endpoint: `${API}/v1/lightning/invoices/<invoice_id>`, headers: { Authorization: 'Bearer $SAFROD_API_KEY' }, note: 'Poll until status is paid. This is real Lightning settlement on regtest; it does not use mainnet funds.' },
    ];
  }

  if (destNetwork.toLowerCase().includes('on-chain')) {
    return [
      post(1, 'Create a regtest deposit address', '/v1/bitcoin/addresses', {
        label: 'flow-builder-demo',
        required_confirmations: 1,
      }, 'Safrod returns an address controlled by the local Bitcoin Core regtest wallet.'),
      { step: 2, title: 'Send regtest bitcoin', description: 'Bitcoin Core regtest', method: '', endpoint: '', headers: {}, isInstructional: true, note: 'Send test coins to the returned address and mine blocks on regtest. The complete local journey is also available with scripts/demo-onchain.sh. No real bitcoin is involved.' },
      { step: 3, title: 'Watch observed deposits', description: '/v1/bitcoin/transactions', method: 'GET', endpoint: `${API}/v1/bitcoin/transactions?limit=20`, headers: { Authorization: 'Bearer $SAFROD_API_KEY' }, note: 'Poll for the deposit and wait for the requested confirmation count.' },
    ];
  }

  const sourceCurrency = source.code === 'KES' ? 'KES' : 'NGN';
  const targetCurrency = destination.code === 'KES' ? 'KES' : 'NGN';
  return [
    post(1, 'Fund the simulated source balance', '/v1/sandbox/balances', {
      currency: sourceCurrency,
      minor_units: 5000000,
    }, 'This creates synthetic test funds only; it does not represent a bank balance.'),
    post(2, 'Create a simulated quote', '/v1/sandbox/quotes', {
      source: { currency: sourceCurrency, minor_units: 500000 },
      target_currency: targetCurrency,
    }, 'The quote and both fiat balances are synthetic. No NGN or KES moves.'),
    post(3, 'Run a simulated payout', '/v1/sandbox/payouts', {
      quote_id: '<quote_id from step 2>',
      scenario: 'success',
    }, 'Use success, failure, or timeout to exercise recovery. Responses are labelled simulated_fiat: true.'),
    { step: 4, title: 'Inspect the payout', description: '/v1/sandbox/payouts/{id}', method: 'GET', endpoint: `${API}/v1/sandbox/payouts/<payout_id>`, headers: { Authorization: 'Bearer $SAFROD_API_KEY' }, note: 'For a real regtest Lightning leg, create an invoice and pass its settlement_invoice to the payout request. The fiat result remains simulated.' },
  ];
}

export function formatCurl(step: ApiStep): string {
  if (step.isInstructional) return step.note ?? '';
  const lines = [`curl -X ${step.method} "${step.endpoint}"`, '  -H "Authorization: Bearer $SAFROD_API_KEY"'];
  if (step.method === 'POST') {
    lines.push('  -H "Content-Type: application/json"');
    if (step.title === 'Pay from the other node') lines.push('  -H "Idempotency-Key: flow-builder-payment-001"');
    if (step.title === 'Run a simulated payout') lines.push('  -H "Idempotency-Key: flow-builder-payout-001"');
    lines.push(`  -d '${JSON.stringify(step.body)}'`);
  }
  return lines.join(' \\\n  ');
}

export function formatStepJson(step: ApiStep): string {
  const { note, description, isInstructional, ...rest } = step;
  void note; void description; void isInstructional;
  return JSON.stringify(rest, null, 2);
}
