import { useCallback, useMemo, useReducer } from 'react';
import { currencies } from '@/data/currencies';
import { cryptoAssets, type CryptoAccountType } from '@/data/crypto';
import type { CurrencySelection } from '@/lib/code-generator';

export type { CurrencySelection };

interface FlowBuilderState {
  send: CurrencySelection | null;
  receive: CurrencySelection | null;
  pickerTarget: 'send' | 'receive' | null;
  audience: 'human' | 'agent';
}

type Action =
  | { type: 'SET_SEND'; payload: CurrencySelection }
  | { type: 'SET_RECEIVE'; payload: CurrencySelection }
  | { type: 'OPEN_PICKER'; payload: 'send' | 'receive' }
  | { type: 'CLOSE_PICKER' }
  | { type: 'SWAP' }
  | { type: 'SET_AUDIENCE'; payload: 'human' | 'agent' };

const initialState: FlowBuilderState = { send: null, receive: null, pickerTarget: null, audience: 'human' };

function reducer(state: FlowBuilderState, action: Action): FlowBuilderState {
  switch (action.type) {
    case 'SET_SEND': return { ...state, send: action.payload, pickerTarget: null };
    case 'SET_RECEIVE': return { ...state, receive: action.payload, pickerTarget: null };
    case 'OPEN_PICKER': return { ...state, pickerTarget: action.payload };
    case 'CLOSE_PICKER': return { ...state, pickerTarget: null };
    case 'SWAP': return { ...state, send: state.receive, receive: state.send };
    case 'SET_AUDIENCE': return { ...state, audience: action.payload };
  }
  return state;
}

export function lookupCurrency(code: string): CurrencySelection | null {
  const fiat = currencies.find((item) => item.code === code);
  if (fiat) {
    return {
      code: fiat.code, type: 'fiat', name: fiat.name, accountType: fiat.accountType,
      accountLabel: fiat.accountLabel, countryCode: fiat.countryCode, jitEligible: false,
      isInternal: false, examplePerson: fiat.examplePerson,
    };
  }
  const crypto = cryptoAssets.find((item) => item.symbol === code);
  if (!crypto) return null;
  const account: CryptoAccountType = crypto.accountTypes[0];
  return {
    code: crypto.symbol, type: 'crypto', name: crypto.name, accountType: account.type,
    accountLabel: account.label, network: account.network, jitEligible: true,
    isInternal: false, examplePerson: crypto.examplePerson,
  };
}

export function useFlowBuilder() {
  const [state, dispatch] = useReducer(reducer, initialState);
  const isComplete = useMemo(() => state.send !== null && state.receive !== null, [state.send, state.receive]);

  const setSend = useCallback((selection: CurrencySelection) => dispatch({ type: 'SET_SEND', payload: selection }), []);
  const setReceive = useCallback((selection: CurrencySelection) => dispatch({ type: 'SET_RECEIVE', payload: selection }), []);
  const openPicker = useCallback((target: 'send' | 'receive') => dispatch({ type: 'OPEN_PICKER', payload: target }), []);
  const closePicker = useCallback(() => dispatch({ type: 'CLOSE_PICKER' }), []);
  const swap = useCallback(() => dispatch({ type: 'SWAP' }), []);
  const setAudience = useCallback((audience: 'human' | 'agent') => dispatch({ type: 'SET_AUDIENCE', payload: audience }), []);

  return { state, isComplete, setSend, setReceive, openPicker, closePicker, swap, setAudience };
}
