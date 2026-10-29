'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'motion/react';
import { Search } from 'lucide-react';
import { cryptoAssets } from '@/data/crypto';
import { currencies } from '@/data/currencies';
import type { CurrencySelection } from '@/lib/code-generator';
import styles from './CurrencyPicker.module.scss';

interface CurrencyPickerProps {
  open: boolean;
  target: 'send' | 'receive' | null;
  onClose: () => void;
  onSelect: (selection: CurrencySelection) => void;
  excludeCode?: string;
}

function makeSelection(code: string): CurrencySelection | null {
  const fiat = currencies.find((item) => item.code === code);
  if (fiat) return {
    code: fiat.code, type: 'fiat', name: fiat.name, accountType: fiat.accountType,
    accountLabel: fiat.accountLabel, countryCode: fiat.countryCode,
    jitEligible: false, isInternal: false, examplePerson: fiat.examplePerson,
  };
  const asset = cryptoAssets.find((item) => item.symbol === code);
  if (!asset) return null;
  const account = asset.accountTypes[0];
  return {
    code: asset.symbol, type: 'crypto', name: asset.name, accountType: account.type,
    accountLabel: account.label, network: account.network, jitEligible: true,
    isInternal: false, examplePerson: asset.examplePerson,
  };
}

export function CurrencyPicker({ open, target, onClose, onSelect, excludeCode }: CurrencyPickerProps) {
  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const choices = useMemo(() => [
    ...cryptoAssets.map((item) => ({ id: item.symbol, name: item.name, meta: item.accountTypes[0].network, group: 'Bitcoin regtest' })),
    ...currencies.map((item) => ({ id: item.code, name: item.name, meta: item.accountLabel, group: 'Sandbox · simulated fiat' })),
  ].filter((item) => item.id !== excludeCode && `${item.id} ${item.name} ${item.meta}`.toLowerCase().includes(query.toLowerCase().trim())), [excludeCode, query]);

  useEffect(() => {
    if (!open) return;
    setQuery('');
    setActiveIndex(0);
  }, [open]);

  const handleKeyDown = useCallback((event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Escape') { event.preventDefault(); onClose(); }
    if (event.key === 'ArrowDown') { event.preventDefault(); setActiveIndex((index) => Math.min(index + 1, choices.length - 1)); }
    if (event.key === 'ArrowUp') { event.preventDefault(); setActiveIndex((index) => Math.max(index - 1, 0)); }
    if (event.key === 'Enter' && choices[activeIndex]) {
      event.preventDefault();
      const selection = makeSelection(choices[activeIndex].id);
      if (selection) onSelect(selection);
    }
  }, [activeIndex, choices, onClose, onSelect]);

  if (typeof document === 'undefined') return null;
  return createPortal(
    <AnimatePresence>
      {open && (
        <div className={styles.pickerLayer}>
          <motion.button className={styles.pickerBackdrop} aria-label="Close selector" onClick={onClose} initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} />
          <motion.div className={styles.pickerDialog} role="dialog" aria-modal="true" aria-label={`Choose ${target === 'send' ? 'source' : 'destination'}`} initial={{ opacity: 0, y: 8, scale: 0.98 }} animate={{ opacity: 1, y: 0, scale: 1 }} exit={{ opacity: 0, y: 8, scale: 0.98 }}>
            <label className={styles.pickerSearch}>
              <Search size={18} />
              <input autoFocus value={query} onChange={(event) => { setQuery(event.target.value); setActiveIndex(0); }} onKeyDown={handleKeyDown} placeholder="Search assets or currencies" />
              <kbd>Esc</kbd>
            </label>
            <div className={styles.pickerList} role="listbox">
              {choices.length ? choices.map((item, index) => (
                <button
                  className={styles.pickerOption}
                  key={item.id}
                  type="button"
                  role="option"
                  aria-selected={index === activeIndex}
                  onMouseEnter={() => setActiveIndex(index)}
                  onClick={() => { const selection = makeSelection(item.id); if (selection) onSelect(selection); }}
                >
                  <span className={styles.pickerGlyph}>{item.id.startsWith('BTC') ? '₿' : item.id.slice(0, 1)}</span>
                  <span className={styles.pickerText}><span className={styles.pickerName}>{item.name}</span><span className={styles.pickerMeta}>{item.meta}</span></span>
                  <span className={styles.pickerCode}>{item.id}</span>
                </button>
              )) : <p className={styles.pickerEmpty}>No matching test asset</p>}
            </div>
            <div className={styles.pickerFoot}><span>Lightning and Bitcoin use local regtest</span><span>Fiat choices are simulated</span></div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
