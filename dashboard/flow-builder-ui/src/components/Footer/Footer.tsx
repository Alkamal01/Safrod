'use client';

import type { ThemePref } from '@/hooks/useTheme';
import { Sun, Moon, Monitor, CodeXml } from 'lucide-react';
import styles from './Footer.module.scss';
import clsx from 'clsx';

interface FooterProps {
  pref: ThemePref;
  setPref: (preference: ThemePref) => void;
}

const OPTIONS = [
  { mode: 'system', label: 'Switch to system theme', Icon: Monitor },
  { mode: 'light', label: 'Switch to light theme', Icon: Sun },
  { mode: 'dark', label: 'Switch to dark theme', Icon: Moon },
] as const;

export function Footer({ pref, setPref }: FooterProps) {
  return (
    <>
      <div className={styles.toggle} role="group" aria-label="Theme preference">
        {OPTIONS.map(({ mode, label, Icon }) => (
          <button
            key={mode}
            type="button"
            aria-label={label}
            aria-pressed={pref === mode}
            className={clsx(styles.toggleItem, pref === mode && styles.toggleItemActive)}
            onClick={() => setPref(mode)}
          >
            <Icon size={14} />
          </button>
        ))}
      </div>
      <div className={styles.trailing}>
        <a href="http://127.0.0.1:8080/docs" className={styles.link}>View API docs</a>
        <a href="https://github.com/Alkamal01/Safrod" className={styles.iconLink} aria-label="Safrod on GitHub">
          <CodeXml size={20} />
        </a>
      </div>
    </>
  );
}
