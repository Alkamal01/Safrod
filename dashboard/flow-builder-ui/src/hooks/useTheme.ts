'use client';

import { useState, useCallback, useEffect, useRef } from 'react';

export type Theme = 'light' | 'dark';
/** The selected theme preference. `system` follows the OS preference. */
export type ThemePref = 'system' | Theme;

const resolve = (pref: ThemePref): Theme =>
  pref === 'system'
    ? window.matchMedia('(prefers-color-scheme: dark)').matches
      ? 'dark'
      : 'light'
    : pref;

const isPref = (v: unknown): v is ThemePref =>
  v === 'system' || v === 'light' || v === 'dark';

export function useTheme() {
  const [pref, setPrefState] = useState<ThemePref>('system');
  const [theme, setThemeState] = useState<Theme>('light');
  const prefRef = useRef(pref);

  const apply = useCallback((p: ThemePref) => {
    const t = resolve(p);
    prefRef.current = p;
    setPrefState(p);
    setThemeState(t);
    // data-theme is the resolved color mode; data-theme-pref is the user choice.
    document.documentElement.setAttribute('data-theme', t);
    document.documentElement.setAttribute('data-theme-pref', p);
  }, []);

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const paramPref = params.get('theme');
    const stored = localStorage.getItem('safrod-theme');
    apply(isPref(paramPref) ? paramPref : isPref(stored) ? stored : 'system');

    // Follow OS theme changes while system preference is selected.
    const mql = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = () => {
      if (prefRef.current === 'system') apply('system');
    };
    mql.addEventListener('change', onChange);
    return () => mql.removeEventListener('change', onChange);
  }, [apply]);

  const setPref = useCallback(
    (p: ThemePref) => {
      apply(p);
      localStorage.setItem('safrod-theme', p);
    },
    [apply],
  );

  return { theme, pref, setPref };
}
