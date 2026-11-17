---
description: >-
  Ask for an image or a video and dedicated media agents generate it, with
  image editing and animation of a reference image, saved in your Files folder.
icon: clapperboard
---

# Image and video generation

OpenHuman can make media, not just read it. Ask for "an image of…", "edit this screenshot to…" or "animate this photo into a short clip", and a dedicated media sub-agent takes over. You need no plugin, no API key and no separate billing.

## What it can do

- **Image generation and editing.** Text-to-image and image editing through hosted GMI models (Seedream for generation, SeedEdit for edits).
- **Video generation.** Text-to-video, or animate a reference image into a clip (Seedance or Veo). Video is asynchronous. The agent starts the render and collects the clip when it is done.
- **Model discovery.** The agent can list the media models available now and pick the right one.

## How it works

The `media_generation` domain (`crates/openhuman-core/src/media/generation/`) gives the agent three tools: generate image, generate video and list models. They use the OpenHuman backend's media-generation provider. The backend owns the provider keys, billing and rate limiting, and your subscription covers it like any other model call.

Images poll every 4 seconds for up to 180 seconds; video polls every 5 seconds for up to 600 seconds. You and the agent see live progress instead of a hung call. Finished files are filed in the Files folder and returned as local file paths, ready to attach, post or edit further.

## Privacy

Prompts and reference media go to the OpenHuman backend and on to the hosted media provider. The in-app capability catalog discloses this (`intelligence.image_generation` and `intelligence.video_generation`, both Beta).

[Privacy mode](../privacy-mode.md) local-only enforcement covers inference providers only. The media tools still call the backend, so avoid them if you need strict no-egress today.

## See also

- [Image tools](image-tools.md): attachments, image metadata and the vision model slot.
- [Native tools](README.md): the full toolbelt.
- [Billing, cost and usage](../billing-and-usage.md): how media jobs are metered.
