import { useCallback, useEffect, useState } from 'react';
import { Link } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';
import {
  openhumanGetConfig,
  openhumanUpdateBrowserSettings,
} from '../../../utils/tauriCommands/config';
import { Alert, AlertDescription } from '../../ui/Alert';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import Card from '../../ui/Card';
import Input from '../../ui/Input';
import Label from '../../ui/Label';
import NativeSelect from '../../ui/NativeSelect';
import Switch from '../../ui/Switch';
import SettingsTabbedPage from '../layout/SettingsTabbedPage';

type BrowserSettings = {
  enabled: boolean;
  headless: boolean;
  viewport_width: number;
  viewport_height: number;
  profile_mode: 'fresh' | 'persistent';
  max_task_steps: number;
  task_timeout_secs: number;
  learn_from_tasks: boolean;
  chrome_path?: string | null;
  profile_path?: string | null;
  download_dir?: string | null;
};

const numericBounds = {
  viewport_width: [320, 3840],
  viewport_height: [240, 2160],
  max_task_steps: [1, 100],
  task_timeout_secs: [5, 600],
} as const;

const defaults: BrowserSettings = {
  enabled: false,
  headless: true,
  viewport_width: 1280,
  viewport_height: 720,
  chrome_path: '',
  profile_mode: 'fresh',
  profile_path: '',
  download_dir: '',
  max_task_steps: 20,
  task_timeout_secs: 120,
  learn_from_tasks: true,
};

/** The fields this panel edits, all accepted by `config.update_browser_settings`. */
const SETTING_KEYS = Object.keys(defaults) as (keyof BrowserSettings)[];

/**
 * Keep only the panel's own fields. The `[browser]` config block also carries
 * legacy keys (`allowed_domains`, `native_*`, `computer_use`, ...), and the
 * core rejects any unknown param, so sending the whole block fails every save.
 */
function pickSettings(source: Partial<Record<keyof BrowserSettings, unknown>>): BrowserSettings {
  const picked: Partial<Record<keyof BrowserSettings, unknown>> = {};
  for (const key of SETTING_KEYS) {
    if (source[key] !== undefined) picked[key] = source[key];
  }
  return { ...defaults, ...(picked as Partial<BrowserSettings>) };
}

type BrowserReadiness = { module_ready: boolean; chrome_ready: boolean; error?: string | null };

export interface BrowserConnectionsPanelProps {
  /** Render only the body, for hosting inside the Computer panel's chip tabs. */
  embedded?: boolean;
}

export default function BrowserConnectionsPanel({
  embedded = false,
}: BrowserConnectionsPanelProps = {}) {
  const { t } = useT();
  const [settings, setSettings] = useState<BrowserSettings>(defaults);
  const [chromeReady, setChromeReady] = useState<boolean | null>(null);
  const [allowedDomains, setAllowedDomains] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');

  const refresh = useCallback(async () => {
    const configResponse = await openhumanGetConfig();
    const config = configResponse.result.config;
    setSettings(pickSettings((config.browser ?? {}) as Partial<BrowserSettings>));
    const httpRequest = (config.http_request ?? {}) as { allowed_domains?: string[] };
    setAllowedDomains(httpRequest.allowed_domains ?? []);
  }, []);

  useEffect(() => {
    let active = true;
    void Promise.resolve().then(() => {
      if (!active) return;
      void refresh().catch(error => {
        if (active) setMessage(error instanceof Error ? error.message : String(error));
      });
    });
    return () => {
      active = false;
    };
  }, [refresh]);

  const save = async () => {
    setBusy(true);
    setMessage('');
    try {
      if (
        Object.entries(numericBounds).some(([key, [min, max]]) => {
          const value = settings[key as keyof typeof numericBounds];
          return !Number.isInteger(value) || value < min || value > max;
        })
      ) {
        setMessage(t('connections.browser.boundsRequired'));
        return;
      }
      await openhumanUpdateBrowserSettings(pickSettings(settings));
      setChromeReady(null);
      await refresh();
      setMessage(t('connections.browser.saved'));
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const testBrowser = async () => {
    setBusy(true);
    setMessage('');
    try {
      // callCoreRpc already unwraps the JSON-RPC `result`.
      const response = await callCoreRpc<BrowserReadiness>({
        method: 'openhuman.modules_browser_check_readiness',
      });
      setChromeReady(response.chrome_ready);
      setMessage(response.error ?? t('connections.browser.readinessChecked'));
    } catch (error) {
      setChromeReady(false);
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const forgetSites = async () => {
    setBusy(true);
    setMessage('');
    try {
      // What finished tasks learned is kept in the workspace, per site.
      const response = await callCoreRpc<{ forgotten: number }>({
        method: 'openhuman.modules_browser_forget_sites',
        params: {},
      });
      setMessage(
        t('connections.browser.sitesForgotten').replace('{count}', String(response.forgotten))
      );
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const field = (key: keyof BrowserSettings, label: string, type: 'text' | 'number' = 'text') => (
    <div className="space-y-1.5" key={key}>
      <Label htmlFor={`browser-${key}`}>{label}</Label>
      <Input
        id={`browser-${key}`}
        type={type}
        min={type === 'number' ? numericBounds[key as keyof typeof numericBounds][0] : undefined}
        max={type === 'number' ? numericBounds[key as keyof typeof numericBounds][1] : undefined}
        value={settings[key] == null ? '' : String(settings[key])}
        onChange={event =>
          setSettings(current => ({
            ...current,
            [key]: type === 'number' ? Number(event.target.value) : event.target.value,
          }))
        }
      />
    </div>
  );

  const body = (
    <div className="max-w-5xl space-y-4 text-sm text-content">
      {!embedded && (
        <Alert variant="warning" role={undefined}>
          <AlertDescription>{t('connections.earlyAlphaNotice')}</AlertDescription>
        </Alert>
      )}
      <Card title={t('connections.browser.chrome')} padded divided={false}>
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div className="flex flex-wrap items-center gap-3">
            <Badge
              variant={
                chromeReady === true ? 'success' : chromeReady === false ? 'warning' : 'neutral'
              }>
              {chromeReady === true
                ? t('connections.browser.chromeReady')
                : chromeReady === false
                  ? t('connections.browser.chromeNotReady')
                  : t('connections.browser.notVerified')}
            </Badge>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button variant="secondary" size="sm" disabled={busy} onClick={testBrowser}>
              {t('connections.browser.testBrowser')}
            </Button>
          </div>
        </div>
        <p className="mt-3 text-xs text-content-muted">{t('connections.browser.chromeHint')}</p>
      </Card>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card title={t('connections.tabs.browser')} padded divided={false}>
          <div className="space-y-5">
            <div className="flex items-center justify-between gap-4">
              <Label htmlFor="browser-enabled">{t('connections.browser.enabled')}</Label>
              <Switch
                id="browser-enabled"
                checked={settings.enabled}
                onCheckedChange={enabled => setSettings(current => ({ ...current, enabled }))}
              />
            </div>
            <div className="flex items-center justify-between gap-4">
              <Label htmlFor="browser-headless">{t('connections.browser.headless')}</Label>
              <Switch
                id="browser-headless"
                checked={settings.headless}
                onCheckedChange={headless => setSettings(current => ({ ...current, headless }))}
              />
            </div>
            <div className="grid gap-4 sm:grid-cols-2">
              {field('viewport_width', t('connections.browser.width'), 'number')}
              {field('viewport_height', t('connections.browser.height'), 'number')}
            </div>
          </div>
        </Card>

        <Card title={t('connections.browser.profileMode')} padded divided={false}>
          <div className="space-y-4">
            <div className="space-y-1.5">
              <Label htmlFor="browser-profile-mode">{t('connections.browser.profileMode')}</Label>
              <NativeSelect
                id="browser-profile-mode"
                className="w-full"
                value={settings.profile_mode ?? 'fresh'}
                onChange={event =>
                  setSettings(current => ({
                    ...current,
                    profile_mode: event.target.value as 'fresh' | 'persistent',
                  }))
                }>
                <option value="fresh">{t('connections.browser.fresh')}</option>
                <option value="persistent">{t('connections.browser.persistent')}</option>
              </NativeSelect>
            </div>
            {settings.profile_mode === 'persistent' &&
              field('profile_path', t('connections.browser.profilePath'))}
            {field('chrome_path', t('connections.browser.chromePath'))}
            {field('download_dir', t('connections.browser.downloadDir'))}
          </div>
        </Card>

        <Card title={t('connections.browser.maxSteps')} padded divided={false}>
          <div className="grid gap-4 sm:grid-cols-2">
            {field('max_task_steps', t('connections.browser.maxSteps'), 'number')}
            {field('task_timeout_secs', t('connections.browser.timeout'), 'number')}
          </div>
          <p className="mt-4 text-xs text-content-muted">
            {t('connections.browser.testInConversation')}
          </p>
        </Card>

        <Card title={t('connections.browser.learning')} padded divided={false}>
          <div className="space-y-4">
            <div className="flex items-center justify-between gap-4">
              <Label htmlFor="browser-learn">{t('connections.browser.learnFromTasks')}</Label>
              <Switch
                id="browser-learn"
                checked={settings.learn_from_tasks}
                onCheckedChange={learn_from_tasks =>
                  setSettings(current => ({ ...current, learn_from_tasks }))
                }
              />
            </div>
            <p className="text-xs text-content-muted">{t('connections.browser.learnHint')}</p>
            <Button variant="secondary" size="sm" disabled={busy} onClick={forgetSites}>
              {t('connections.browser.forgetSites')}
            </Button>
          </div>
        </Card>

        <Card title={t('connections.browser.allowedWebsites')} padded divided={false}>
          <p className="text-content-muted">{t('connections.browser.sharedPolicy')}</p>
          <p className="mt-3 break-words font-medium">
            {allowedDomains.length
              ? allowedDomains.join(', ')
              : t('connections.browser.noneAllowed')}
          </p>
          <Button asChild variant="tertiary" size="sm" className="mt-3 px-0">
            <Link to="/connections?tab=search">{t('connections.browser.manageWebsites')}</Link>
          </Button>
        </Card>
      </div>

      <div className="flex flex-wrap items-center justify-between gap-3">
        <span />
        <Button disabled={busy} onClick={save}>
          {t('connections.browser.save')}
        </Button>
      </div>
      {message && (
        <Alert variant="info" role="status">
          <AlertDescription>{message}</AlertDescription>
        </Alert>
      )}
    </div>
  );

  if (embedded) return body;

  return (
    <SettingsTabbedPage
      title={t('connections.tabs.browser')}
      description={t('connections.browser.description')}>
      {body}
    </SettingsTabbedPage>
  );
}
