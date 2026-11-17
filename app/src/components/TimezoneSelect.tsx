import { useEffect, useMemo, useState } from 'react';

import { useT } from '../lib/i18n/I18nContext';
import {
  openhumanGetUserTimezone,
  openhumanUpdateUserTimezone,
  type UserTimezoneSettings,
} from '../utils/tauriCommands';
import { SELECT_CLASS } from './LanguageSelect';

/** The option that follows the device's zone instead of a chosen one. */
const FOLLOW_DEVICE = '';

/** Every zone the picker offers: UTC, then the runtime's IANA zones. */
function ianaZones(): string[] {
  try {
    return ['UTC', ...Intl.supportedValuesOf('timeZone')];
  } catch {
    return ['UTC'];
  }
}

/** Whether a saved value is a zone the picker can offer. */
function isOfferedZone(zone: string | null | undefined): zone is string {
  return !!zone && ianaZones().includes(zone);
}

interface TimezoneSelectProps {
  /** Accessible label for the underlying <select>. */
  ariaLabel?: string;
}

/**
 * Settings → Account time zone picker. The user's IANA zone is what the
 * assistant reads dates in ("yesterday", "last Saturday"); "Use device time
 * zone" follows the machine instead. Loads and saves through core config
 * (`config_get_user_timezone` / `config_update_user_timezone`).
 */
const TimezoneSelect = ({ ariaLabel }: TimezoneSelectProps) => {
  const { t } = useT();
  const [settings, setSettings] = useState<UserTimezoneSettings | null>(null);
  const [failed, setFailed] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let live = true;
    openhumanGetUserTimezone()
      .then(response => {
        if (live) setSettings(response.result);
      })
      .catch(() => {
        if (live) setFailed(true);
      });
    return () => {
      live = false;
    };
  }, []);

  // A saved value that is not an offered zone shows as "follow the device",
  // which is what core resolves it to.
  const chosen = isOfferedZone(settings?.timezone) ? settings?.timezone : null;
  const zones = useMemo(() => ianaZones(), []);

  const change = async (value: string) => {
    const timezone = value === FOLLOW_DEVICE ? null : value;
    setFailed(false);
    // The picker is disabled until this save settles, so two picks cannot
    // race and land out of order. Nothing is shown as chosen until core has
    // stored it, and then exactly what it stored (no re-read).
    setSaving(true);
    try {
      await openhumanUpdateUserTimezone(timezone);
      setSettings(current =>
        current ? { ...current, timezone, effective: timezone ?? current.device ?? 'UTC' } : current
      );
    } catch {
      // The save may still have been stored (a lost response, say): show
      // what core actually holds, and report a failure only if it is not
      // the pick. If core cannot be read either, report the failure.
      try {
        const stored = (await openhumanGetUserTimezone()).result;
        setSettings(stored);
        setFailed(stored.timezone !== timezone);
      } catch {
        setFailed(true);
      }
    } finally {
      setSaving(false);
    }
  };

  const device = settings?.device ?? t('settings.timezoneUnknownDevice');
  return (
    <div className="flex flex-col items-end gap-1">
      <select
        value={chosen ?? FOLLOW_DEVICE}
        onChange={e => void change(e.target.value)}
        disabled={!settings || saving}
        aria-label={ariaLabel ?? t('settings.timezone')}
        data-testid="timezone-select"
        className={SELECT_CLASS}>
        <option value={FOLLOW_DEVICE}>
          {t('settings.timezoneDevice').replace('{zone}', device)}
        </option>
        {zones.map(zone => (
          <option key={zone} value={zone}>
            {zone.replace(/_/g, ' ')}
          </option>
        ))}
      </select>
      {failed && (
        <span
          role="alert"
          className="text-xs font-medium text-red-600"
          data-testid="timezone-error">
          {t('settings.timezoneSaveFailed')}
        </span>
      )}
    </div>
  );
};

export default TimezoneSelect;
