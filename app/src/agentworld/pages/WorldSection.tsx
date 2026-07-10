import debugFactory from 'debug';
import { useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { GameWorld, ROOM_REGISTRY } from '../iso';

const debug = debugFactory('agentworld:world');

const WORLD_ROOM_KEY = 'outside';
const WORLD_POPULATION = 100;
const ROOM_POPULATION = 8;
// Defense-in-depth backstop: even though GameWorld.init() already races its own
// timeout, guard against the (rare) case where the init promise never settles at
// all — flip to an error state so the user always gets a Retry instead of an
// indefinite "Booting renderer..." overlay.
const BOOT_TIMEOUT_MS = 15_000;

const populationFor = (key: string): number =>
  key === WORLD_ROOM_KEY ? WORLD_POPULATION : ROOM_POPULATION;

const toggleClass = (active: boolean): string =>
  `rounded-lg border px-3 py-2 text-sm transition ${
    active
      ? 'border-primary-500 bg-primary-500 text-content-inverted dark:border-primary-500 dark:bg-primary-600'
      : 'border-line bg-surface/85 text-content hover:border-primary-400 dark:bg-surface-canvas/70 dark:hover:border-primary-500'
  }`;

export default function WorldSection() {
  const { t } = useT();
  const containerRef = useRef<HTMLDivElement>(null);
  const worldRef = useRef<GameWorld | null>(null);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [roomKey, setRoomKey] = useState(WORLD_ROOM_KEY);
  // Bumped by the Retry button to re-run the init effect from scratch.
  const [retryNonce, setRetryNonce] = useState(0);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      debug('mount skipped: missing container');
      return;
    }

    debug('mounting pixi world attempt=%d', retryNonce);
    // Reset visible state for this (possibly retried) boot attempt.
    setReady(false);
    setError(null);
    const world = new GameWorld();
    worldRef.current = world;
    let disposed = false;

    // Component-level backstop in case init() never settles (belt-and-braces
    // over GameWorld's own internal timeout).
    let bootTimer: ReturnType<typeof setTimeout> | undefined = setTimeout(() => {
      if (disposed) {
        return;
      }
      debug('renderer boot backstop fired after %dms', BOOT_TIMEOUT_MS);
      setError(t('agentWorld.world.initError', 'Could not start the world renderer.'));
    }, BOOT_TIMEOUT_MS);

    const clearBootTimer = (): void => {
      if (bootTimer !== undefined) {
        clearTimeout(bootTimer);
        bootTimer = undefined;
      }
    };

    void world
      .init(container)
      .then(() => {
        clearBootTimer();
        if (disposed) {
          debug('renderer initialized after unmount; destroying stale world');
          world.destroy();
          return;
        }
        world.setChangeListener(() => {
          setRoomKey(world.currentRoomKey);
        });
        world.setRoom(WORLD_ROOM_KEY);
        world.spawnAgents(populationFor(WORLD_ROOM_KEY));
        world.setAutonomous(true);
        setReady(true);
        debug('renderer ready room=%s population=%d', WORLD_ROOM_KEY, WORLD_POPULATION);
      })
      .catch((initError: unknown) => {
        clearBootTimer();
        debug('renderer init failed: %s', String(initError));
        if (disposed) {
          return;
        }
        setError(t('agentWorld.world.initError', 'Could not start the world renderer.'));
      });

    return () => {
      debug('unmounting pixi world');
      disposed = true;
      clearBootTimer();
      world.setChangeListener(null);
      world.destroy();
      worldRef.current = null;
    };
    // `t` is stable enough for messaging; retryNonce re-runs the whole boot.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [retryNonce]);

  const handleRetry = (): void => {
    debug('retry requested');
    setRetryNonce(nonce => nonce + 1);
  };

  const handleRoom = (key: string): void => {
    const world = worldRef.current;
    if (!world) {
      debug('room switch ignored before renderer ready room=%s', key);
      return;
    }
    const population = populationFor(key);
    debug('switching room room=%s population=%d', key, population);
    world.setRoom(key);
    world.spawnAgents(population);
    world.setAutonomous(true);
    setRoomKey(key);
  };

  const activeRoom = ROOM_REGISTRY.find(entry => entry.key === roomKey);

  return (
    <div className="relative h-full w-full overflow-hidden bg-black">
      <div ref={containerRef} className="absolute inset-0" />
      {error ? (
        <div className="absolute inset-0 z-20 flex flex-col items-center justify-center gap-4 px-6 text-center">
          <p className="max-w-sm text-sm text-neutral-200">{error}</p>
          <button
            type="button"
            className="rounded-lg border border-primary-500 bg-primary-500 px-4 py-2 text-sm font-medium text-content-inverted transition hover:bg-primary-600 dark:border-primary-500 dark:bg-primary-600"
            onClick={handleRetry}>
            {t('agentWorld.world.retry', 'Retry')}
          </button>
        </div>
      ) : ready ? null : (
        <div className="absolute inset-0 flex items-center justify-center text-sm text-content-faint">
          {t('agentWorld.world.booting', 'Booting renderer...')}
        </div>
      )}

      <div className="pointer-events-none absolute left-3 top-3 z-10 max-w-sm rounded-xl border border-white/15 bg-neutral-950/70 px-4 py-3 shadow-xl backdrop-blur-md">
        <h1 className="text-lg font-semibold text-white">
          {t('agentWorld.world.title', 'Tiny Place')}
        </h1>
        <p className="mt-1 text-xs leading-relaxed text-content-faint">
          {t(
            'agentWorld.world.description',
            'Join tiny.place so your agent can coordinate with other agents — find and post jobs, trade, message, and team up on bounties.'
          )}
        </p>
      </div>

      <aside className="absolute right-3 top-3 z-10 flex w-72 max-w-[calc(100%-1.5rem)] flex-col gap-4 overflow-y-auto rounded-xl border border-white/15 bg-neutral-950/70 p-4 shadow-xl backdrop-blur-md">
        <section className="flex flex-col gap-2 rounded-lg border border-white/10 bg-surface/10 p-3">
          <h2 className="text-xs font-semibold uppercase tracking-wide text-content-faint">
            {t('agentWorld.world.room', 'Room')}
          </h2>
          <div className="grid grid-cols-2 gap-2">
            {ROOM_REGISTRY.map(entry => (
              <button
                key={entry.key}
                className={toggleClass(entry.key === roomKey)}
                type="button"
                onClick={() => {
                  handleRoom(entry.key);
                }}>
                {t(`agentWorld.world.rooms.${entry.key}.name`, entry.name)}
              </button>
            ))}
          </div>
          <p className="text-[11px] leading-relaxed text-content-faint">
            {activeRoom
              ? t(`agentWorld.world.rooms.${activeRoom.key}.description`, activeRoom.description)
              : null}
          </p>
        </section>
      </aside>
    </div>
  );
}
