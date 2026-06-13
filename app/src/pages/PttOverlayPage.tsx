import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';

import { useT } from '../lib/i18n/I18nContext';

export function PttOverlayPage() {
  const { t } = useT();
  const [active, setActive] = useState(false);

  useEffect(() => {
    let off: UnlistenFn | undefined;
    let cancelled = false;
    listen<{ active: boolean }>('ptt-overlay://active', e => {
      setActive(Boolean(e.payload?.active));
    })
      .then(fn => {
        if (cancelled) fn();
        else off = fn;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      off?.();
    };
  }, []);

  return (
    <div
      data-testid="ptt-overlay-root"
      data-active={active}
      style={{
        width: '160px',
        height: '56px',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 8,
        background: 'rgba(20, 20, 24, 0.85)',
        borderRadius: 12,
        color: '#fff',
        fontFamily: 'Inter, system-ui, sans-serif',
        fontSize: 12,
        userSelect: 'none',
        pointerEvents: 'none',
      }}>
      <span
        aria-hidden
        style={{
          width: 10,
          height: 10,
          borderRadius: 999,
          background: active ? '#ff4d4f' : '#666',
          boxShadow: active ? '0 0 6px #ff4d4f' : undefined,
          transition: 'all 120ms ease',
        }}
      />
      {/* TODO(T13): i18n keys pttOverlay.listening / pttOverlay.idle added in T13 */}
      <span>{active ? t('pttOverlay.listening') : t('pttOverlay.idle')}</span>
    </div>
  );
}
