export interface PopularFlow { label: string; sendCode: string; receiveCode: string }

export const popularFlows: PopularFlow[] = [
  { label: 'Lightning → Lightning · regtest', sendCode: 'BTC-LN', receiveCode: 'BTC-LN' },
  { label: 'Wallet → Bitcoin deposit · regtest', sendCode: 'BTC-CHAIN', receiveCode: 'BTC-CHAIN' },
  { label: 'NGN → KES · simulated', sendCode: 'NGN', receiveCode: 'KES' },
];
