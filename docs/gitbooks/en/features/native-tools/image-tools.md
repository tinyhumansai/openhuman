---
description: >-
  Show the agent an image: attachments the model can see, local image
  metadata, and which model handles vision.
icon: image
---

# Image tools

This page is about getting an image into a turn so the model can look at it. To make images instead, see [Image and video generation](media-generation.md).

## Attaching an image

Attach a file in the composer, or point the agent at a path. What happens next depends on the model and the route, not on a setting.

- If the model accepts images and the transport supports them, the image goes up natively as image content.
- If not, it degrades instead of failing. The agent gets a bounded readout, or a reference to the file with its path and metadata. It can still reason about the file and tell you what it cannot see.

Uploads stay in the acting workspace across a restart. The ordered references stay in the transcript, so a later turn can still refer to "the second screenshot".

## `image_info`

`image_info` is a read-only tool. It reads a local image's metadata (format, dimensions, size) and can return the bytes as base64 text for a model that wants them inline. Use it when the question is "what is this file". It does not need a vision model.

## Which model sees it

Vision has its own provider slot, so you can route it separately from chat:

```toml
vision_provider = "…"   # a provider id, or leave unset to use the chat provider
```

If a configured vision model turns out to be chat-only, the call returns a named error. It never silently swaps in another model. There is no default local vision model, so a local setup has to name one. [Local models and BYOK](../model-routing/local-and-byok-models.md) lists the ones worth trying.

## Delegation

When a turn needs to look at several images, or study one closely, the agent usually hands it to the built-in vision sub-agent. That keeps the main conversation from carrying every pixel of context. The agent decides this itself. You do not have to ask.

## See also

- [Image and video generation](media-generation.md): making images and video.
- [Chat](../chat.md): where attachments live in the UI.
- [Automatic model routing](../model-routing/README.md): how the vision slot is resolved.
