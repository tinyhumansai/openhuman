# image

High-level contracts for image-capable agent runtimes.

This module does not execute image generation or pixel inspection directly. It
defines the stable model-facing contracts that provider/runtime adapters can
expose when image capabilities are available.

## Responsibilities

- Define the `image_generation` contract for hosted raster image creation and
  edits.
- Define the `view_image` contract for loading local image files into
  model-visible image context.
- Gate contract exposure by runtime support and local policy:
  - generated image writes
  - local image reads
- Render concise prompt guidance for agents when image tools are available.
- Keep schema/serialization contracts covered by focused Rust tests.

## Module Shape

| File | Role |
| --- | --- |
| `mod.rs` | Export-only module entrypoint. |
| `types.rs` | Shared descriptors, permission/config types, and gating helpers. |
| `image_generation.rs` | `image_generation` schema and output-format contract. |
| `image_view.rs` | `view_image` schema and detail-level contract. |
| `prompt.rs` | Agent prompt guidance for enabled image tools. |
| `tests.rs` | Contract-level e2e tests across config, schemas, and prompt output. |

## Notes

Existing lower-level image helpers remain separate:

- `image_info` reads metadata/base64 text.
- Browser screenshot tooling captures page images.
- Multimodal `[IMAGE:...]` preparation normalizes image references for
  image-capable providers.

Future execution adapters should depend on these contracts and live next to the
runtime/provider implementation that actually performs the hosted call or model
attachment.
