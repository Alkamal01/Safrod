import type { CurrencySelection } from './code-generator';

export type ActionColor = 'fiat' | 'btc' | 'stable';

export interface ActionCard {
  text: string;
  color: ActionColor;
  gradientFrom?: ActionColor;
}

export interface FlowNode {
  id: string;
  type: 'endpoint' | 'switch';
  label: string;
  sublabel?: string;
  flag?: string;
  cryptoIcon?: string;
  isInternal: boolean;
  actionCards: ActionCard[];
}

export interface ConnectorLabel { text: string }
export interface FlowPath { nodes: FlowNode[]; connectorLabels: ConnectorLabel[] }

export function buildFlowPath(source: CurrencySelection, destination: CurrencySelection): FlowPath {
  const sourceColor: ActionColor = source.type === 'fiat' ? 'fiat' : 'btc';
  const destColor: ActionColor = destination.type === 'fiat' ? 'fiat' : 'btc';
  const simulated = source.type === 'fiat' && destination.type === 'fiat';
  const compatible = simulated
    ? source.code !== destination.code
    : source.type === destination.type && source.network === destination.network;
  const middle: FlowNode = {
    id: 'safrod',
    type: 'switch',
    label: 'Safrod API',
    sublabel: compatible ? (simulated ? 'Fiat leg simulated' : 'Bitcoin regtest') : 'Choose a supported test pair',
    isInternal: true,
    actionCards: [{
      text: !compatible ? 'No matching API recipe' : simulated ? 'Sandbox quote and payout' : destination.network?.includes('on-chain') ? 'Track confirmations' : 'Create and settle invoice',
      color: simulated ? 'fiat' : 'btc',
    }],
  };
  const nodes: FlowNode[] = [
    {
      id: 'source', type: 'endpoint', label: source.name, sublabel: source.accountLabel,
      cryptoIcon: source.type === 'crypto' ? '/crypto/btc.svg' : undefined,
      flag: source.type === 'fiat' ? source.countryCode : undefined,
      isInternal: false, actionCards: [{ text: `Start with ${source.code}`, color: sourceColor }],
    },
    middle,
    {
      id: 'destination', type: 'endpoint', label: destination.name, sublabel: destination.accountLabel,
      cryptoIcon: destination.type === 'crypto' ? '/crypto/btc.svg' : undefined,
      flag: destination.type === 'fiat' ? destination.countryCode : undefined,
      isInternal: false, actionCards: [{ text: `Deliver to ${destination.code}`, color: destColor }],
    },
  ];
  return { nodes, connectorLabels: [{ text: 'Request' }, { text: simulated ? 'Simulated fiat' : 'Regtest' }] };
}
