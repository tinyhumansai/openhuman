/**
 * Resolve mascot face/viseme state against a *specific* mascot's `stateEngine`.
 *
 * The bundled-asset helpers in `../riveMaps` assume the `tiny_mascot.riv`
 * vocabulary. Manifest mascots each ship their own poses, viseme codes, and
 * channels, so these helpers constrain every value to what the selected asset
 * actually accepts — setting an out-of-vocabulary enum on a Rive state machine
 * is a silent no-op, which would freeze the mouth or pose.
 */
import type { MascotFace } from '../Ghosty';
import { faceToPose, toRiveVisemeCode } from '../riveMaps';
import type { MascotManifestEntry, MascotStateEngine } from './types';

/** Every pose value this mascot's state machine accepts (cycle + logical states). */
export function availablePoses(engine: MascotStateEngine): Set<string> {
  return new Set<string>([...engine.idlePoseCycle, ...Object.values(engine.states)]);
}

/**
 * Map a {@link MascotFace} to a pose this mascot can actually play. Manifest
 * logical mappings win first, then the shared face→pose vocabulary, then a
 * guaranteed logical state (`thinking` for thinking-ish faces, otherwise
 * `idle`) when the mascot's asset doesn't carry that specific flourish.
 */
export function resolveFaceToPose(face: MascotFace, engine: MascotStateEngine): string {
  const logical = engine.states[face];
  if (logical && availablePoses(engine).has(logical)) return logical;

  const desired = faceToPose(face);
  if (availablePoses(engine).has(desired)) return desired;
  return desired === 'thinking' ? engine.states.thinking : engine.states.idle;
}

/** The resting mouth code for this mascot (`sil` when present, else its first code). */
export function restVisemeCode(engine: MascotStateEngine): string {
  return engine.visemeCodes.find(code => code.toLowerCase() === 'sil') ?? engine.visemeCodes[0];
}

/**
 * Normalise an incoming Oculus/ElevenLabs viseme code to one this mascot's
 * mouth enum accepts. Unknown or out-of-vocabulary codes resolve to the
 * resting (closed) mouth so the mouth never sticks on a no-op value.
 */
export function resolveVisemeCode(code: string, engine: MascotStateEngine): string {
  const normalised = toRiveVisemeCode(code);
  const candidates = [code, normalised];
  return (
    engine.visemeCodes.find(candidate =>
      candidates.some(alias => candidate.toLowerCase() === alias.toLowerCase())
    ) ?? restVisemeCode(engine)
  );
}

/**
 * Pick a random idle flourish pose for this mascot, excluding the resting
 * `idle` pose (and optionally a just-played one) so the same flourish never
 * fires twice in a row. `rng` is injectable for deterministic tests.
 */
export function pickIdleFlourish(
  engine: MascotStateEngine,
  exclude?: string,
  rng: () => number = Math.random
): string {
  const rest = engine.states.idle;
  const pool = engine.idlePoseCycle.filter(p => p !== rest && p !== exclude);
  const choices = pool.length > 0 ? pool : engine.idlePoseCycle.filter(p => p !== rest);
  if (choices.length === 0) return rest;
  const idx = Math.min(choices.length - 1, Math.floor(rng() * choices.length));
  return choices[idx];
}

/** Initial values for every channel: its `default`, falling back to `values[0]`. */
export function initialChannelValues(engine: MascotStateEngine): Record<string, string> {
  const out: Record<string, string> = {};
  for (const channel of engine.channels ?? []) {
    out[channel.key] = channel.default ?? channel.values[0];
  }
  return out;
}

/** Convenience: pull a mascot entry's state engine. */
export function stateEngineOf(entry: MascotManifestEntry): MascotStateEngine {
  return entry.stateEngine;
}
