import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useState } from 'react';
import { LuCopy, LuMinus, LuSquare, LuX } from 'react-icons/lu';

import { useT } from '../../../lib/i18n/I18nContext';
import { isTauri } from '../../../utils/tauriCommands/common';
import { maximizeWindow, minimizeWindow, quitApp } from '../../../utils/tauriCommands/window';

export const WINDOWS_WINDOW_CONTROLS_WIDTH = 138;

export function isWindowsDesktop(): boolean {
  return isTauri() && typeof navigator !== 'undefined' && /win/i.test(navigator.platform);
}

function WindowControls() {
  const { t } = useT();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const window = getCurrentWindow();
    let mounted = true;
    void window.isMaximized().then(value => {
      if (mounted) setMaximized(value);
    });
    const unlisten = window.onResized(() => {
      void window.isMaximized().then(value => {
        if (mounted) setMaximized(value);
      });
    });
    return () => {
      mounted = false;
      void unlisten.then(stop => stop());
    };
  }, []);

  const buttonClass =
    'flex h-8 w-[46px] items-center justify-center text-content-primary transition-colors hover:bg-content-faint/60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-content-primary';

  return (
    <div
      className="absolute right-0 top-0 z-50 flex h-8 select-none"
      style={{ width: WINDOWS_WINDOW_CONTROLS_WIDTH }}>
      <button
        type="button"
        className={buttonClass}
        aria-label={t('window.minimize')}
        title={t('window.minimize')}
        onClick={() => void minimizeWindow()}>
        <LuMinus aria-hidden="true" className="h-4 w-4" />
      </button>
      <button
        type="button"
        className={buttonClass}
        aria-label={t(maximized ? 'window.restore' : 'window.maximize')}
        title={t(maximized ? 'window.restore' : 'window.maximize')}
        onClick={() => void maximizeWindow()}>
        {maximized ? (
          <LuCopy aria-hidden="true" className="h-3.5 w-3.5" />
        ) : (
          <LuSquare aria-hidden="true" className="h-3.5 w-3.5" />
        )}
      </button>
      <button
        type="button"
        className={`${buttonClass} hover:bg-red-600 hover:text-white`}
        aria-label={t('common.close')}
        title={t('common.close')}
        onClick={() => void quitApp()}>
        <LuX aria-hidden="true" className="h-4 w-4" />
      </button>
    </div>
  );
}

export default function WindowsWindowControls() {
  if (!isWindowsDesktop()) return null;
  return <WindowControls />;
}
