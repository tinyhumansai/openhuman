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
  /** Resolved style of the canvas; only set once `connect()` ran. */
  computedCanvasStyle?: CSSStyleDeclaration;
  /**
   * Latched by `disconnect()`. Every async entry point below bails on it so a
   * queued frame/timer never touches a canvas React already unmounted (#5160).
   */
  destroyed: boolean;
  play(): void;
  pause(): void;
  disconnect(): void;
  initGradient(selector: string): this;
  toggleColor(index: number): void;
  updateFrequency(freq: number): void;
  /**
   * Lifecycle internals. Public only because the teardown regression tests
   * (#5160) drive them directly — the React wrapper uses `initGradient`,
   * `play`, `pause` and `disconnect`.
   */
  init(): void;
  initGradientColors(): void;
  initMesh(): void;
  resize(): void;
  waitForCssVars(): void;
  addIsLoadedClass(): void;
}
