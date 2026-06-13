# Image Tools

OpenHuman's image contract gives agents a stable way to reason about
image generation and local image inspection without tying the prompt surface to a
single provider runtime.

## Scope

The contract lives in the top-level `src/openhuman/image/` module and
currently covers two model-facing tools:

| Tool | Purpose | Permission | Output |
| --- | --- | --- | --- |
| `image_generation` | Generate or edit raster images from a prompt. | Write | Local generated-media artifact paths. |
| `view_image` | Load a local image file into model-visible image context. | Read-only | Image content visible to the model. |

This layer is intentionally high level. Existing lower-level tools still own
their concrete behavior:

- `image_info` reads local image metadata and optional base64 text.
- Browser screenshot tooling captures pages and writes image files.
- Agent multimodal preparation normalizes `[IMAGE:...]` markers for providers
  that accept image data.

The image layer defines names, schemas, gating, and prompt rules so
agents can make consistent decisions as runtimes add direct support.

## `image_generation`

`image_generation` is a hosted provider capability. The Rust core should not
pretend to be an image renderer when no provider supports it. When enabled, the
runtime should:

1. Validate any `input_image_path` through the same local-file policy used for
   image viewing.
2. Send the prompt and optional edit image to the hosted image provider.
3. Persist returned bytes under a session-scoped generated-media root, or under
   an approved caller-provided `output_path`.
4. Return saved artifact paths so the final assistant answer can reference them.

The schema includes `prompt`, optional `output_path`, optional `size`, optional
`input_image_path`, and `output_format` (`png`, `webp`, `jpeg`).

## `view_image`

`view_image` loads pixels from a local file into model-visible context. Use it
when text metadata is insufficient: screenshots, UI review, OCR, diagrams,
charts, visual diffs, and generated-image inspection.

The runtime must keep local file boundaries explicit:

- Allow paths in the approved workspace.
- Allow paths created during the current session.
- Allow paths explicitly referenced by the user or trusted tool output.
- Deny paths outside policy, and do not silently attach unrelated local images.

The schema includes `path` and optional `detail` (`auto`, `high`, `original`).
Use `original` only when full-resolution inspection is necessary.

## Prompt Guidance

Prompt rendering should include image guidance only when at least one
media tool is enabled. The guidance should tell agents:

- Use `view_image` when pixels are needed, not for ordinary file metadata.
- Use `image_generation` for requested raster image creation or edits.
- Provide an output path when the destination matters.
- Mention generated artifact paths in final answers.
- Respect local image boundaries before attaching a file to model context.

## Tests

The module has focused Rust tests for:

- JSON schema shape for `image_generation`.
- JSON schema shape for `view_image`.
- Independent gating of generation vs local viewing.
- End-to-end contract rendering from config to specs and prompt guidance.

Future runtime PRs should add provider-specific execution tests next to the
runtime adapter, not in the hosted contract module.
