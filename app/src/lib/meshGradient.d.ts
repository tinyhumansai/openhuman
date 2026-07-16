export interface GradientConfig {
  playing: boolean;
}

export class Gradient {
  el?: HTMLCanvasElement;
  conf?: GradientConfig;
  /**
   * The WebGL mesh. Only set once `connect()` acquires a GL context and builds
   * the geometry; stays `undefined` on no-GPU / headless environments. Callers
   * must check it before `play()`, since the animation loop dereferences
   * `mesh.material` and would throw when it is absent (#3524).
   */
  mesh?: unknown;
  play(): void;
  pause(): void;
  disconnect(): void;
  initGradient(selector: string): this;
  toggleColor(index: number): void;
  updateFrequency(freq: number): void;
}
