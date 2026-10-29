export interface CryptoAccountType { type: string; label: string; network: string }
export interface ExamplePerson { fullName: string; nationality: string }
export interface CryptoAsset { symbol: string; name: string; accountTypes: CryptoAccountType[]; examplePerson: ExamplePerson }

export const cryptoAssets: CryptoAsset[] = [
  { symbol: 'BTC-LN', name: 'Bitcoin · Lightning', accountTypes: [{ type: 'LIGHTNING_REGTEST', label: 'Lightning invoice', network: 'Lightning regtest' }], examplePerson: { fullName: 'Safrod Demo', nationality: 'NG' } },
  { symbol: 'BTC-CHAIN', name: 'Bitcoin · on-chain', accountTypes: [{ type: 'BITCOIN_REGTEST', label: 'Deposit address', network: 'Bitcoin on-chain regtest' }], examplePerson: { fullName: 'Safrod Demo', nationality: 'NG' } },
];
