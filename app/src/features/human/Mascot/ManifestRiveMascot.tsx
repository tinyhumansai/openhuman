import {
  Fit,
  Layout,
  useRive,
  useViewModel,
  useViewModelInstance,
  useViewModelInstanceColor,
  useViewModelInstanceEnum,
} from '@rive-app/react-webgl2';
import debug from 'debug';
import { type FC, useEffect, useRef, useState } from 'react';

import type { MascotFace } from './Ghosty';
import { loadManifestRiv } from './manifest/manifestService';
import { pickIdleFlourish, resolveFaceToPose, resolveVisemeCode } from './manifest/stateEngine';
import type {
  MascotManifestChannel,
  MascotManifestEntry,
  MascotStateEngine,
} from './manifest/types';
import { MASCOT_STATE_MACHINE } from './riveMaps';
import { RiveMascot } from './RiveMascot';

const log = debug('human:mascot:manifest-rive');

/** Idle dwell before the mascot drifts into a flourish (ms), randomised. */
const AMBIENT_IDLE_MIN_MS = 6_000;
const AMBIENT_IDLE_MAX_MS = 12_000;
/** How long a flourish is held before returning to the resting pose (ms). */
const AMBIENT_HOLD_MIN_MS = 2_500;
const AMBIENT_HOLD_MAX_MS = 5_000;
/** Fallback channel auto-cycle interval when the manifest omits one (ms). */
const CHANNEL_CYCLE_FALLBACK_MS = 2_500;

function randBetween(min: number, max: number): number {
  return min + Math.random() * (max - min);
}

/**
 * Map a value to the asset's actual enum option, case-insensitively. Rive
 * enums are case-sensitive and silently ignore an unrecognised value — so if
 * our resolved code (`oh`) doesn't exactly match the asset's option (`OH`),
 * the mouth/pose freezes. The runtime exposes the real option list; we match
 * against it and return the exact-cased option. Falls back to the input when
 * the options aren't loaded yet or there's no match.
 */
function matchEnumValue(value: string, options: readonly string[] | undefined): string {
  if (!options || options.length === 0) return value;
  return options.find(o => o.toLowerCase() === value.toLowerCase()) ?? value;
}

const RIVE_LAYOUT = new Layout({ fit: Fit.Contain });

export interface ManifestRiveMascotProps {
  /** The manifest entry to render. Its runtime `.riv` is loaded + cached. */
  entry: MascotManifestEntry;
  face?: MascotFace;
  size?: number | string;
  primaryColor?: number;
  secondaryColor?: number;
  /** Raw Oculus 15-set viseme code; normalised to this mascot's vocabulary. */
  visemeCode?: string;
  /** Drift through this mascot's idle pose cycle + auto-cycle its channels. */
  idlePoseRotation?: boolean;
}

/**
 * Render a manifest mascot from its loaded `.riv` buffer. Split out from the
 * loader so every Rive hook runs against a present buffer (calling the Rive
 * hooks with no source then swapping in a buffer mid-mount destabilises the
 * runtime). The parent keys this by mascot id so a new selection remounts it.
 */
const ManifestRiveStage: FC<{
  buffer: ArrayBuffer;
  engine: MascotStateEngine;
  channels: MascotManifestChannel[];
  face: MascotFace;
  size: number | string;
  primaryColor?: number;
  secondaryColor?: number;
  visemeCode: string;
  idlePoseRotation: boolean;
}> = ({
  buffer,
  engine,
  channels,
  face,
  size,
  primaryColor,
  secondaryColor,
  visemeCode,
  idlePoseRotation,
}) => {
  const { rive, RiveComponent } = useRive({
    buffer,
    stateMachines: MASCOT_STATE_MACHINE,
    autoplay: true,
    layout: RIVE_LAYOUT,
  });

  const viewModel = useViewModel(rive, { useDefault: true });
  const vmInstance = useViewModelInstance(viewModel, { useDefault: true, rive });
  const { setValue: setPose, values: poseEnumValues } = useViewModelInstanceEnum(
    'pose',
    vmInstance
  );
  const { setValue: setMouthVisemeCode, values: visemeEnumValues } = useViewModelInstanceEnum(
    'mouthVisemeCode',
    vmInstance
  );
  const { setValue: setPrimaryColor } = useViewModelInstanceColor('primaryColor', vmInstance);
  const { setValue: setSecondaryColor } = useViewModelInstanceColor('secondaryColor', vmInstance);

  // `useViewModelInstanceEnum(...).values` returns a NEW array reference on
  // every render. Depending on that identity in an effect causes the effect to
  // re-run each render → setValue updates the hook's internal state → re-render
  // → loop ("Maximum update depth exceeded"). So we keep the arrays in refs
  // (read inside effects) and gate re-runs on a *content* key that only changes
  // when the asset's option list actually changes (i.e. once, when it loads).
  const poseValuesRef = useRef<readonly string[] | undefined>(poseEnumValues);
  poseValuesRef.current = poseEnumValues;
  const visemeValuesRef = useRef<readonly string[] | undefined>(visemeEnumValues);
  visemeValuesRef.current = visemeEnumValues;
  const poseEnumKey = (poseEnumValues ?? []).join('');
  const visemeEnumKey = (visemeEnumValues ?? []).join('');

  // One-time visibility into what the asset actually accepts vs. what we send.
  const loggedEnumsRef = useRef(false);
  useEffect(() => {
    if (loggedEnumsRef.current) return;
    if (visemeEnumKey.length > 0 || poseEnumKey.length > 0) {
      loggedEnumsRef.current = true;
      log('asset enums — pose=%o viseme=%o', poseValuesRef.current, visemeValuesRef.current);
    }
  }, [poseEnumKey, visemeEnumKey]);

  const basePose = resolveFaceToPose(face, engine);
  const restPose = engine.states.idle;

  // Driven (face-derived) pose. A real activity pose always wins; the resting
  // pose is what the idle scheduler below is free to override.
  useEffect(() => {
    setPose(matchEnumValue(basePose, poseValuesRef.current));
  }, [basePose, setPose, poseEnumKey]);

  // Idle pose rotation, scoped to this mascot's idlePoseCycle. Same self-
  // rescheduling shape as RiveMascot; only runs while enabled AND resting.
  const setPoseRef = useRef(setPose);
  setPoseRef.current = setPose;
  useEffect(() => {
    if (!idlePoseRotation || basePose !== restPose) return;
    let timer: number | undefined;
    let current = restPose;
    const toRest = () => {
      current = restPose;
      setPoseRef.current(matchEnumValue(restPose, poseValuesRef.current));
      timer = window.setTimeout(toFlourish, randBetween(AMBIENT_IDLE_MIN_MS, AMBIENT_IDLE_MAX_MS));
    };
    const toFlourish = () => {
      current = pickIdleFlourish(engine, current === restPose ? undefined : current);
      log('idle flourish → %s', current);
      setPoseRef.current(matchEnumValue(current, poseValuesRef.current));
      timer = window.setTimeout(toRest, randBetween(AMBIENT_HOLD_MIN_MS, AMBIENT_HOLD_MAX_MS));
    };
    timer = window.setTimeout(toFlourish, randBetween(AMBIENT_IDLE_MIN_MS, AMBIENT_IDLE_MAX_MS));
    return () => {
      if (timer !== undefined) window.clearTimeout(timer);
      setPoseRef.current(matchEnumValue(restPose, poseValuesRef.current));
    };
  }, [idlePoseRotation, basePose, restPose, engine]);

  useEffect(() => {
    const code = resolveVisemeCode(visemeCode, engine);
    setMouthVisemeCode(matchEnumValue(code, visemeValuesRef.current));
  }, [visemeCode, engine, setMouthVisemeCode, visemeEnumKey]);

  useEffect(() => {
    if (primaryColor !== undefined) setPrimaryColor(primaryColor);
  }, [primaryColor, setPrimaryColor]);

  useEffect(() => {
    if (secondaryColor !== undefined) setSecondaryColor(secondaryColor);
  }, [secondaryColor, setSecondaryColor]);

  return (
    <div
      style={{
        width: typeof size === 'number' ? `${size}px` : size,
        height: typeof size === 'number' ? `${size}px` : size,
      }}
      data-face={face}>
      <RiveComponent />
      {channels.map(channel => (
        <ChannelDriver
          key={channel.key}
          channel={channel}
          vmInstance={vmInstance}
          autoCycle={idlePoseRotation}
        />
      ))}
    </div>
  );
};

/**
 * Drives one optional enum channel (e.g. `eyes`) onto the view model. Each
 * channel is its own component so the rules-of-hooks count stays stable, and
 * auto-cycles its value on a timer when the manifest marks it cyclable and the
 * mascot is in its "feel alive" mode.
 */
const ChannelDriver: FC<{
  channel: MascotManifestChannel;
  vmInstance: ReturnType<typeof useViewModelInstance>;
  autoCycle: boolean;
}> = ({ channel, vmInstance, autoCycle }) => {
  const { setValue } = useViewModelInstanceEnum(channel.key, vmInstance);
  const [value, setVal] = useState<string>(channel.default ?? channel.values[0]);

  useEffect(() => {
    if (value != null) setValue(value);
  }, [value, setValue]);

  useEffect(() => {
    if (!autoCycle || !channel.cycle?.enabled || channel.values.length < 2) return;
    const interval = channel.cycle.intervalMs ?? CHANNEL_CYCLE_FALLBACK_MS;
    const sequential = channel.cycle.order === 'sequential';
    let index = 0;
    const timer = window.setInterval(() => {
      if (sequential) {
        index = (index + 1) % channel.values.length;
        setVal(channel.values[index]);
      } else {
        setVal(channel.values[Math.floor(Math.random() * channel.values.length)]);
      }
    }, interval);
    return () => window.clearInterval(timer);
  }, [autoCycle, channel]);

  return null;
};

/**
 * Load a manifest mascot's `.riv` and render it. While the buffer resolves —
 * or if it fails — the bundled default mascot keeps the stage alive and still
 * lip-syncs, so a slow GitHub fetch never blanks the Human page.
 */
export const ManifestRiveMascot: FC<ManifestRiveMascotProps> = ({
  entry,
  face = 'idle',
  size = '100%',
  primaryColor,
  secondaryColor,
  visemeCode = 'sil',
  idlePoseRotation = false,
}) => {
  const [loadState, setLoadState] = useState<{
    entryId: string;
    buffer: ArrayBuffer | null;
    failed: boolean;
  }>({ entryId: entry.id, buffer: null, failed: false });

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const buf = await loadManifestRiv(entry);
        if (!cancelled) setLoadState({ entryId: entry.id, buffer: buf, failed: false });
      } catch (err) {
        if (!cancelled) {
          log('failed to load mascot %s: %o', entry.id, err);
          setLoadState({ entryId: entry.id, buffer: null, failed: true });
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [entry]);

  const fallbackProps = { face, size, primaryColor, secondaryColor, visemeCode, idlePoseRotation };
  const currentBuffer = loadState.entryId === entry.id ? loadState.buffer : null;
  const currentFailed = loadState.entryId === entry.id && loadState.failed;
  if (currentFailed || !currentBuffer) return <RiveMascot key="default" {...fallbackProps} />;

  return (
    <ManifestRiveStage
      key={`buf-${entry.id}`}
      buffer={currentBuffer}
      engine={entry.stateEngine}
      channels={entry.stateEngine.channels ?? []}
      face={face}
      size={size}
      primaryColor={primaryColor}
      secondaryColor={secondaryColor}
      visemeCode={visemeCode}
      idlePoseRotation={idlePoseRotation}
    />
  );
};
