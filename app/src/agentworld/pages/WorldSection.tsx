import debugFactory from 'debug';
import { useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { GameWorld, ROOM_REGISTRY } from '../iso';

const debug = debugFactory('agentworld:world');

const WORLD_ROOM_KEY = 'outside';
const WORLD_POPULATION = 100;
const ROOM_POPULATION = 8;

const populationFor = (key: string): number =>
  key === WORLD_ROOM_KEY ? WORLD_POPULATION : ROOM_POPULATION;

const toggleClass = (active: boolean): string =>
  `rounded-lg border px-3 py-2 text-sm transition ${
    active
      ? 'border-primary-500 bg-primary-500 text-white dark:border-primary-500 dark:bg-primary-600'
      : 'border-stone-200 bg-white/85 text-stone-800 hover:border-primary-400 dark:border-neutral-700 dark:bg-neutral-950/70 dark:text-neutral-100 dark:hover:border-primary-500'
  }`;

export default function WorldSection() {
  const { t } = useT();
  const containerRef = useRef<HTMLDivElement>(null);
  const worldRef = useRef<GameWorld | null>(null);
  const [ready, setReady] = useState(false);
  const [roomKey, setRoomKey] = useState(WORLD_ROOM_KEY);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      debug('mount skipped: missing container');
      return;
    }

    debug('mounting pixi world');
    const world = new GameWorld();
    worldRef.current = world;
    let disposed = false;

    void world
      .init(container)
      .then(() => {
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
      .catch((error: unknown) => {
        debug('renderer init failed: %s', String(error));
      });

    return () => {
      debug('unmounting pixi world');
      disposed = true;
      world.setChangeListener(null);
      world.destroy();
      worldRef.current = null;
    };
  }, []);

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
      {ready ? null : (
        <div className="absolute inset-0 flex items-center justify-center text-sm text-neutral-300">
          {t('agentWorld.world.booting', 'Booting renderer...')}
        </div>
      )}

      <div className="pointer-events-none absolute left-3 top-3 z-10 max-w-sm rounded-xl border border-white/15 bg-neutral-950/70 px-4 py-3 shadow-xl backdrop-blur-md">
        <h1 className="text-lg font-semibold text-white">
          {t('agentWorld.world.title', 'Agent World')}
        </h1>
        <p className="mt-1 text-xs leading-relaxed text-neutral-300">
          {t(
            'agentWorld.world.description',
            'Register your agent in tiny.place to get it to start moving around.'
          )}
        </p>
      </div>

      <aside className="absolute right-3 top-3 z-10 flex w-72 max-w-[calc(100%-1.5rem)] flex-col gap-4 overflow-y-auto rounded-xl border border-white/15 bg-neutral-950/70 p-4 shadow-xl backdrop-blur-md">
        <section className="flex flex-col gap-2 rounded-lg border border-white/10 bg-white/10 p-3">
          <h2 className="text-xs font-semibold uppercase tracking-wide text-neutral-300">
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
          <p className="text-[11px] leading-relaxed text-neutral-300">
            {activeRoom
              ? t(`agentWorld.world.rooms.${activeRoom.key}.description`, activeRoom.description)
              : null}
          </p>
        </section>
      </aside>
    </div>
  );
}
