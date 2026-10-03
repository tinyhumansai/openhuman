/**
 * The Engine picker on its own, loading its own engine state — for hosts that
 * are not the Memory page (the onboarding wizard's memory step).
 *
 * debug logging: DEBUG=openhuman:memory:engine-setup
 */
import debug from 'debug';
import { useEffect, useState } from 'react';

import { type EngineState, memoryEngineGet } from '../../services/api/memoryApi';
import MemoryEngineTab from './MemoryEngineTab';

const log = debug('openhuman:memory:engine-setup');

const OFF: EngineState = { engine: null, has_key: false, status: 'off', fetch_modes: [] };

export default function MemoryEngineSetup() {
  const [state, setState] = useState<EngineState | null>(null);

  useEffect(() => {
    let cancelled = false;
    memoryEngineGet()
      .then(next => {
        if (!cancelled) setState(next);
      })
      .catch(err => {
        log('engine_get failed: %o', err);
        if (!cancelled) setState(OFF);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return <MemoryEngineTab state={state} onStateChange={setState} embedded />;
}
