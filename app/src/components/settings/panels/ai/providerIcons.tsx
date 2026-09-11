/*
 * Brand marks for provider rows and the add-provider dialog.
 *
 * `react-icons` supplies the Simple Icons marks already available in the app.
 * Providers absent from that installed set use locally bundled, traceable
 * assets, so provider marks render offline and never add a runtime network
 * request.
 *
 * Coverage is extended with those bundled assets. A provider with no traceable
 * mark still keeps
 * the lettered swatch, which is why `providerIcon` returns `null` instead of a
 * placeholder glyph — the caller already renders a good fallback, and a generic
 * cloud icon on twelve rows carries less information than twelve letters.
 */
import { createElement, type ReactElement } from 'react';
import type { IconType } from 'react-icons';
import { LuSparkles } from 'react-icons/lu';
import {
  SiAnthropic,
  SiApple,
  SiGooglegemini,
  SiHuggingface,
  SiMistralai,
  SiNvidia,
  SiOllama,
  SiOpenai,
  SiVercel,
  SiX,
} from 'react-icons/si';

import cerebrasLogo from '../../../../assets/provider-icons/cerebras.svg';
import deepinfraLogo from '../../../../assets/provider-icons/deepinfra.svg';
import deepseekLogo from '../../../../assets/provider-icons/deepseek.svg';
import fireworksLogo from '../../../../assets/provider-icons/fireworks.svg';
import gmiLogo from '../../../../assets/provider-icons/gmi.ico';
import groqLogo from '../../../../assets/provider-icons/groq.svg';
import kilocodeLogo from '../../../../assets/provider-icons/kilocode.ico';
import lmstudioLogo from '../../../../assets/provider-icons/lmstudio.svg';
import minimaxLogo from '../../../../assets/provider-icons/minimax.svg';
import modelscopeLogo from '../../../../assets/provider-icons/modelscope.svg';
import moonshotLogo from '../../../../assets/provider-icons/moonshot.svg';
import novitaLogo from '../../../../assets/provider-icons/novita.svg';
import openrouterLogo from '../../../../assets/provider-icons/openrouter.svg';
import orcarouterLogo from '../../../../assets/provider-icons/orcarouter.ico';
import stepfunLogo from '../../../../assets/provider-icons/stepfun.svg';
import sumopodLogo from '../../../../assets/provider-icons/sumopod.ico';
import togetherLogo from '../../../../assets/provider-icons/together.svg';
import veniceLogo from '../../../../assets/provider-icons/venice.ico';
import zaiLogo from '../../../../assets/provider-icons/zai.ico';
import { cn } from '../../../../lib/cn';

/**
 * Provider slug to brand mark. Keys are the slugs in `builtinCloudProviders.ts`
 * and the local-runtime slugs, so a rename there surfaces here as a silently
 * missing icon rather than a wrong one — `providerIconCoverage` in the tests
 * pins the keys against the real provider list.
 */
const PROVIDER_ICONS: Record<string, IconType> = {
  openhuman: LuSparkles,
  openai: SiOpenai,
  // Codex signs in as an OpenAI credential and is stored under `openai`, so it
  // never reaches this map under its own name.
  anthropic: SiAnthropic,
  'claude-code': SiAnthropic,
  google: SiGooglegemini,
  mistral: SiMistralai,
  huggingface: SiHuggingface,
  nvidia: SiNvidia,
  'vercel-ai-gateway': SiVercel,
  xai: SiX,
  ollama: SiOllama,
  omlx: SiApple,
};

/** Locally bundled marks for providers absent from the installed icon set. */
type ProviderAsset = { src: string; monochrome: boolean };

const PROVIDER_ASSETS: Record<string, ProviderAsset> = {
  cerebras: { src: cerebrasLogo, monochrome: true },
  deepinfra: { src: deepinfraLogo, monochrome: true },
  openrouter: { src: openrouterLogo, monochrome: true },
  deepseek: { src: deepseekLogo, monochrome: true },
  fireworks: { src: fireworksLogo, monochrome: true },
  gmi: { src: gmiLogo, monochrome: false },
  groq: { src: groqLogo, monochrome: true },
  kilocode: { src: kilocodeLogo, monochrome: false },
  lmstudio: { src: lmstudioLogo, monochrome: true },
  minimax: { src: minimaxLogo, monochrome: true },
  modelscope: { src: modelscopeLogo, monochrome: true },
  moonshot: { src: moonshotLogo, monochrome: true },
  novita: { src: novitaLogo, monochrome: true },
  orcarouter: { src: orcarouterLogo, monochrome: false },
  stepfun: { src: stepfunLogo, monochrome: true },
  sumopod: { src: sumopodLogo, monochrome: false },
  together: { src: togetherLogo, monochrome: true },
  venice: { src: veniceLogo, monochrome: false },
  zai: { src: zaiLogo, monochrome: false },
};

/**
 * The rendered brand mark for a provider slug, or `null` when we ship none.
 *
 * Returns an ELEMENT, not a component. A caller that did
 * `const Icon = providerIcon(slug)` and rendered `<Icon />` would be defining a
 * component during render as far as React is concerned (and the lint rule that
 * catches it is right: it gives the icon a fresh identity on every render, so
 * React cannot reconcile it). Handing back an element keeps that impossible at
 * the call site.
 */
export const providerIcon = (slug: string, className: string): ReactElement | null => {
  const icon = PROVIDER_ICONS[slug];
  if (icon) return createElement(icon, { className: cn(className, 'text-content-inverted') });
  const asset = PROVIDER_ASSETS[slug];
  return asset ? (
    <img
      src={asset.src}
      alt=""
      aria-hidden
      className={cn(className, 'object-contain', asset.monochrome && 'brightness-0 invert')}
    />
  ) : null;
};

/** Slugs with a mark — exported for the coverage test, not for rendering. */
export const PROVIDER_ICON_SLUGS = [
  ...Object.keys(PROVIDER_ICONS),
  ...Object.keys(PROVIDER_ASSETS),
];

export default providerIcon;
