# Remotion video

<p align="center">
  <a href="https://github.com/remotion-dev/logo">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://github.com/remotion-dev/logo/raw/main/animated-logo-banner-dark.apng">
      <img alt="Animated Remotion Logo" src="https://github.com/remotion-dev/logo/raw/main/animated-logo-banner-light.gif">
    </picture>
  </a>
</p>

Welcome to your Remotion project!

## Commands

**Install Dependencies**

```console
pnpm install
```

**Start Preview**

```console
pnpm dev
```

**Render a single variant** (produces `out/<CompositionId>.mov` — transparent ProRes 4444)

```console
pnpm render GhostyWave
```

**Render all variants**

```console
pnpm render:all
```

**Render runtime mascot assets for the desktop app** (writes transparent animated WebP files for `yellow`, `burgundy`, `black`, `navy`, and `green` to `app/public/generated/remotion/`)

> Requires a system `ffmpeg` binary on `PATH` for frame extraction. Install via `apt install ffmpeg`, `brew install ffmpeg`, or `choco install ffmpeg`.

```console
pnpm render:runtime-assets
```

**Upgrade Remotion**

```console
pnpm exec remotion upgrade
```

## Docs

Get started with Remotion by reading the [fundamentals page](https://www.remotion.dev/docs/the-fundamentals).

## Help

We provide help on our [Discord server](https://discord.gg/6VzzNDwUwV).

## Issues

Found an issue with Remotion? [File an issue here](https://github.com/remotion-dev/remotion/issues/new).

## License

Note that for some entities a company license is needed. [Read the terms here](https://github.com/remotion-dev/remotion/blob/main/LICENSE.md).
