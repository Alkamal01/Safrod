'use client';

import { useCallback } from 'react';
import { useFlowBuilder, lookupCurrency } from '@/hooks/useFlowBuilder';
import { useTheme } from '@/hooks/useTheme';
import { usePanelCollapse } from '@/hooks/usePanelCollapse';
import { Header } from '@/components/Header/Header';
import { InputCard, CardChevron } from '@/components/InputCard/InputCard';
import { PopularFlows } from '@/components/PopularFlows/PopularFlows';
import { CurrencyPicker } from '@/components/CurrencyPicker/CurrencyPicker';
import { EmptyCanvas } from '@/components/EmptyCanvas/EmptyCanvas';
import { FlowPanel } from '@/components/FlowPanel/FlowPanel';
import { CodePanel } from '@/components/CodePanel/CodePanel';
import { Footer } from '@/components/Footer/Footer';
import { ArrowRight } from 'lucide-react';
import styles from './page.module.scss';

export default function Home() {
  const builder = useFlowBuilder();
  const { theme, pref, setPref } = useTheme();
  const { flowExpanded, codeExpanded, toggleFlow } = usePanelCollapse();
  const handleThemeChange = useCallback((value: 'system' | 'light' | 'dark') => setPref(value), [setPref]);
  const { state, isComplete } = builder;

  return (
    <main className={styles.layout} data-theme-mode={theme}>
      <aside className={styles.sidebar}>
        <div className={styles.sidebarHeader}>
          <a href="/dashboard" className={styles.brand}>◈ <span>Safrod</span></a>
          <a href="http://127.0.0.1:8080/docs" className={styles.headerLink}>View API docs</a>
        </div>

        <div className={styles.sidebarContent}>
          <div className={styles.sidebarContentInner}>
            <Header />
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--spacing-md)' }}>
              <div style={{ display: 'flex', flexDirection: 'column', isolation: 'isolate' }}>
                <InputCard label="Source" selection={state.send} onCardClick={() => builder.openPicker('send')} />
                <div style={{ position: 'relative', height: '8px' }}>
                  <CardChevron onSwap={state.send || state.receive ? builder.swap : undefined} />
                </div>
                <InputCard label="Destination" selection={state.receive} onCardClick={() => builder.openPicker('receive')} />
              </div>
            </div>
            {isComplete && (
              <a className={styles.mobileViewBtn} href="#flow-result">
                View flow and code <ArrowRight size={16} />
              </a>
            )}
            <div style={{ marginTop: isComplete ? '10px' : '24px' }}>
              <PopularFlows onSelect={(sendCode, receiveCode) => {
                const source = lookupCurrency(sendCode);
                const destination = lookupCurrency(receiveCode);
                if (source) builder.setSend(source);
                if (destination) builder.setReceive(destination);
              }} />
            </div>
          </div>
        </div>

        <div className={styles.sidebarFooter}>
          <Footer pref={pref} setPref={handleThemeChange} />
        </div>
      </aside>

      <section className={styles.rightPanel} id="flow-result">
        {!isComplete ? (
          <EmptyCanvas />
        ) : (
          <div className={styles.rightPanelScroll}>
            <FlowPanel send={state.send!} receive={state.receive!} expanded={flowExpanded} onToggle={toggleFlow} />
            <CodePanel
              send={state.send!}
              receive={state.receive!}
              fundingModel="pre-funded"
              sourceRail={null}
              destRail={null}
              audience={state.audience}
              onAudienceChange={builder.setAudience}
              expanded={codeExpanded}
            />
          </div>
        )}
      </section>

      <CurrencyPicker
        open={state.pickerTarget !== null}
        target={state.pickerTarget}
        excludeCode={state.pickerTarget === 'receive' && state.send?.type === 'fiat'
          ? state.send.code
          : state.pickerTarget === 'send' && state.receive?.type === 'fiat'
            ? state.receive.code
            : undefined}
        onClose={builder.closePicker}
        onSelect={(selection) => {
          if (state.pickerTarget === 'send') builder.setSend(selection);
          else builder.setReceive(selection);
        }}
      />
    </main>
  );
}
