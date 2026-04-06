export interface GradientConfig {
  playing: boolean;
}

export class Gradient {
  el?: HTMLCanvasElement;
  conf?: GradientConfig;
  play(): void;
  pause(): void;
  disconnect(): void;
  initGradient(selector: string): this;
  toggleColor(index: number): void;
  updateFrequency(freq: number): void;
}
