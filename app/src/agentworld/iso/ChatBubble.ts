/**
 * Hyper-lightweight chat bubble.
 *
 * Text is rendered with {@link BitmapText} against a pre-installed bitmap font,
 * so speaking never uploads a fresh text texture to the GPU — glyphs are drawn
 * from a cached atlas. The rounded background is a {@link NineSliceSprite} over
 * one shared texture, which means a bubble of any width costs no extra memory.
 *
 * Each bubble owns its own lifecycle (pop-in, hold, fade-out) and reports when
 * it is finished so the world can recycle it without stalling the render loop.
 */
import { BitmapText, Container, NineSliceSprite, Sprite } from 'pixi.js';

import type { BakedTexture } from './textures';

const HORIZONTAL_PADDING = 11;
const VERTICAL_PADDING = 7;
const SLICE_INSET = 14;
const MAX_TEXT_WIDTH = 168;
const POP_IN_MS = 130;
const FADE_OUT_MS = 320;
const DEFAULT_HOLD_MS = 3200;

interface ChatBubbleOptions {
  background: BakedTexture;
  tail: BakedTexture;
  fontName: string;
  text: string;
  durationMs?: number;
}

type Phase = 'in' | 'hold' | 'out' | 'done';

export class ChatBubble extends Container {
  private phase: Phase = 'in';
  private elapsedMs = 0;
  private readonly holdMs: number;
  private readonly bubbleWidth: number;
  private readonly bubbleHeight: number;

  public constructor(options: ChatBubbleOptions) {
    super();
    this.holdMs = options.durationMs ?? DEFAULT_HOLD_MS;

    const label = new BitmapText({
      text: options.text,
      style: {
        fontFamily: options.fontName,
        fontSize: 15,
        wordWrap: true,
        wordWrapWidth: MAX_TEXT_WIDTH,
        align: 'center',
      },
    });

    this.bubbleWidth = Math.max(SLICE_INSET * 2, Math.ceil(label.width) + HORIZONTAL_PADDING * 2);
    this.bubbleHeight = Math.max(SLICE_INSET * 2, Math.ceil(label.height) + VERTICAL_PADDING * 2);

    const background = new NineSliceSprite({
      texture: options.background.texture,
      leftWidth: SLICE_INSET,
      topHeight: SLICE_INSET,
      rightWidth: SLICE_INSET,
      bottomHeight: SLICE_INSET,
    });
    background.width = this.bubbleWidth;
    background.height = this.bubbleHeight;

    const tail = new Sprite(options.tail.texture);
    tail.anchor.set(0.5, 0);
    tail.position.set(this.bubbleWidth / 2, this.bubbleHeight - 1);

    label.anchor.set(0.5, 0.5);
    label.position.set(this.bubbleWidth / 2, this.bubbleHeight / 2);

    this.addChild(background, tail, label);

    // Pivot at the tail tip so positioning the bubble at the agent's head
    // makes the tail point straight down at it.
    this.pivot.set(this.bubbleWidth / 2, this.bubbleHeight + 8);
    this.alpha = 0;
    this.scale.set(0.7);
  }

  /** Advance the bubble's lifecycle. Returns `false` once it is finished. */
  public update(deltaMs: number): boolean {
    this.elapsedMs += deltaMs;
    switch (this.phase) {
      case 'in': {
        const amount = Math.min(1, this.elapsedMs / POP_IN_MS);
        this.alpha = amount;
        this.scale.set(0.7 + amount * 0.3);
        if (amount >= 1) {
          this.phase = 'hold';
          this.elapsedMs = 0;
        }
        return true;
      }
      case 'hold': {
        if (this.elapsedMs >= this.holdMs) {
          this.phase = 'out';
          this.elapsedMs = 0;
        }
        return true;
      }
      case 'out': {
        const amount = Math.min(1, this.elapsedMs / FADE_OUT_MS);
        this.alpha = 1 - amount;
        this.y -= deltaMs * 0.012;
        if (amount >= 1) {
          this.phase = 'done';
        }
        return true;
      }
      default:
        return false;
    }
  }

  /** Begin an early dismissal (e.g. when the agent speaks again). */
  public dismiss(): void {
    if (this.phase !== 'out' && this.phase !== 'done') {
      this.phase = 'out';
      this.elapsedMs = 0;
    }
  }
}
