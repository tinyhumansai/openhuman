import { QRCodeSVG } from 'qrcode.react';
import { type ReactNode, useState } from 'react';

import Button from '../../../../components/ui/Button';
import { PopoverContent, PopoverRoot, PopoverTrigger } from '../../../../components/ui/Popover';
import { useT } from '../../../../lib/i18n/I18nContext';
import { openUrl } from '../../../../utils/openUrl';
import { classifyHref } from '../../utils/format';
import type { McpUiLink } from './types';

async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

function displayUrl(url: string): string {
  return url.length > 64 ? `${url.slice(0, 61)}…` : url;
}

/** A QR code for `url`, the URL itself and a copy button. */
export function QrHandoff({ url, hint }: { url: string; hint: string }) {
  const { t } = useT();
  const [copied, setCopied] = useState(false);
  return (
    <div className="flex flex-col items-center gap-2" data-testid="mcp-ui-qr">
      <p className="text-xs text-content-muted text-center">{hint}</p>
      <div className="rounded-lg p-2 border border-line" style={{ backgroundColor: '#ffffff' }}>
        <QRCodeSVG value={url} size={168} level="M" bgColor="#ffffff" fgColor="#1c1917" />
      </div>
      <span className="max-w-[220px] break-all text-center font-mono text-[11px] text-content-secondary">
        {displayUrl(url)}
      </span>
      <Button
        size="xs"
        variant="secondary"
        analyticsId="mcp-ui-copy-link"
        onClick={() => {
          void copyText(url).then(setCopied);
        }}>
        {copied ? t('conversations.mcpUi.copied') : t('conversations.mcpUi.copyLink')}
      </Button>
    </div>
  );
}

function QrPopoverButton({ url, label, hint }: { url: string; label: string; hint: string }) {
  return (
    <PopoverRoot>
      <PopoverTrigger asChild>
        <Button size="xs" variant="secondary" analyticsId="mcp-ui-show-qr">
          {label}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start">
        <QrHandoff url={url} hint={hint} />
      </PopoverContent>
    </PopoverRoot>
  );
}

/**
 * Actions for the links a tool result offered. `http(s)` links can be opened
 * in the browser or handed to a phone; app links (`upi://`, `phonepe://`) are
 * only ever handed to a phone, never opened here.
 */
export function LinkActions({ links }: { links: McpUiLink[] }) {
  const { t } = useT();
  const usable = links.filter(link => {
    const kind = classifyHref(link.url);
    return kind === 'external' || kind === 'handoff';
  });
  if (usable.length === 0) return null;
  return (
    <ul className="flex flex-col gap-2" aria-label={t('conversations.mcpUi.linksLabel')}>
      {usable.map(link => {
        const external = classifyHref(link.url) === 'external';
        return (
          <li
            key={link.url}
            className="flex flex-wrap items-center gap-2 rounded-lg border border-line bg-surface-muted px-2.5 py-2"
            data-testid="mcp-ui-link">
            <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-content-secondary">
              {displayUrl(link.url)}
            </span>
            {external ? (
              <Button
                size="xs"
                variant="primary"
                analyticsId="mcp-ui-open-link"
                onClick={() => {
                  void openUrl(link.url).catch(() => undefined);
                }}>
                {t('conversations.mcpUi.openInBrowser')}
              </Button>
            ) : null}
            <QrPopoverButton
              url={link.url}
              label={
                external ? t('conversations.mcpUi.showQr') : t('conversations.mcpUi.openOnPhone')
              }
              hint={t('conversations.mcpUi.scanToOpen')}
            />
          </li>
        );
      })}
    </ul>
  );
}

/**
 * A markdown link to an app scheme: clicking shows the QR code instead of
 * trying to open it on this machine.
 */
export function ExternalSchemeLink({ href, children }: { href: string; children?: ReactNode }) {
  const { t } = useT();
  return (
    <PopoverRoot>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="cursor-pointer underline wrap-break-word wrap-anywhere text-start"
          data-testid="handoff-link">
          {children}
        </button>
      </PopoverTrigger>
      <PopoverContent align="start">
        <QrHandoff url={href} hint={t('conversations.mcpUi.scanToOpen')} />
      </PopoverContent>
    </PopoverRoot>
  );
}
