---
description: >-
  Ask for an image or a video and dedicated media agents generate it: image
  generation and editing (Seedream / SeedEdit), plus text-to-video and
  animate-a-reference-image (Seedance / Veo), saved straight into your
  workspace.
icon: clapperboard
---

# Image & Video Generation

OpenHuman can *make* media, not just read it. Ask the assistant to "generate an image of…", "edit this screenshot to…", or "animate this photo into a short clip" and a dedicated media sub-agent takes over. No plugin, no API key, no separate billing.

## What it can do

* **Image generation & editing.** Text-to-image and image editing through hosted GMI models (**Seedream** for generation, **SeedEdit** for edits).
* **Video generation.** Text-to-video, or animate a reference image into a clip (**Seedance** / **Veo**). Video is asynchronous: the agent kicks off the render and collects the clip when it's done.
* **Model discovery.** The agent can list the currently available media models and pick the right one for the job.

## How it works

The `media_generation` domain (`src/openhuman/media_generation/`) exposes three agent tools (generate image, generate video, list models) backed by the OpenHuman backend's media-generation provider. The backend owns the provider keys, billing, and rate limiting; your subscription covers it like any other model call.

The tools submit the job and then poll on a 4-second cadence (up to 180 s for images, 420 s for video), so the agent, and you, get live progress instead of a hung call. Finished artifacts are downloaded into the agent's `generated-media/` folder in your workspace and returned as local file paths, ready to attach, post, or edit further.

## Privacy

Prompts and reference media for these tools are sent to the OpenHuman backend and on to the hosted media provider. This is disclosed in the in-app capability catalog (`intelligence.image_generation` / `intelligence.video_generation`, both Beta). Note that [Privacy Mode](../privacy-mode.md)'s local-only enforcement currently covers **inference providers only**. The media tools still call the backend, so avoid using them if you need strict no-egress today. Extending enforcement to integrations and network tools is a planned later slice.

## See also

* [Image Tools](image-tools.md) covers the *vision* side: reading and analyzing images.
* [Available Tools](./) lists the full native toolbelt.
* [Billing, Cost & Usage](../billing-and-usage.md) explains how media jobs are metered.
