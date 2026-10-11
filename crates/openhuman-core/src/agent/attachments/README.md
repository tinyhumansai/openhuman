# Workspace attachments

This module owns OpenHuman's upload storage and attachment routing policy.
TinyAgents owns generic byte resolution, bounded archive inspection, faithful
PNG optimization, and durable transcript parts. TinyInference owns provider
capability and serialization contracts. TinyDocs owns document extraction and
PDF rendering; OpenHuman calls it through the shared bus contract.

## Intake and durable references

Web chat stages `[FILE:…]` and `[IMAGE:…]` sources before prompt scanning,
history, memory, or queue persistence. `threads.message_append` also stages user
uploads before writing a durable conversation row and removes upload bytes,
posters, and previews from attachment metadata. The normal-send frontend uses
the compact content returned by this append call for the subsequent chat turn,
so that turn reuses the saved originals. Each original is written to
`<action_dir>/uploads/<thread-id>/<uuid>/<sanitized-filename>`. Unsafe thread
identifiers are hashed; the original display filename is retained separately
from the sanitized storage leaf. Exclusive creation prevents overwriting another
upload. Write failures reject the request rather than accepting an upload whose
bytes were lost.

Queued follow-ups are submitted immediately. The frontend temporarily keeps
their raw upload markers in nonpersisted Redux state until the current reply
finishes and the follow-up can be appended in conversation order. The queue
ingress stages the originals; the later history append may stage a second copy
because it receives the raw markers retained by the frontend. Core and frontend
queue previews use the same caption or original filenames, preserving their
correspondence when a queued item is cancelled.

Source counts, byte limits, and remote-fetch permissions come from the core
multimodal configuration. Local reads and destinations use the acting-tool
filesystem policy, including symlink checks and the always-forbidden credential
and internal-state boundaries. External channel input cannot gain local file
reads through delegation or provider preparation; its disabled attachment
configuration remains authoritative.

The staged message carries compact `[ATTACHMENT:…]` metadata: a workspace-relative
path, filename, MIME type, and original size. The relative path also works under
the acting workspace's `/workspace` sandbox mount. Originals have no transient
image-stash expiry and remain available to terminal tools and child agents that
share that workspace.

`message_convert` lifts these references into typed image, audio, video, or
document input blocks while preserving source order and captions. The document
carrier also handles arbitrary binaries. Durable transcript parts retain paths
or URLs; base64 payloads belong to ephemeral provider requests. Context
enrichment prefixes text without flattening media, and replay restores the typed
parts.

## Provider preparation

The provider decorator prepares a request copy. It selects native media only
when the actual selected model is known to accept the modality **and** the
underlying transport supports its MIME type and source representation. Unknown
model capabilities do not imply multimodal support. Uploaded local references
are read under policy and encoded for that request; the durable reference and
original file remain unchanged. A native PDF follows this path without calling
TinyDocs.

Factory-created models receive the scoped runtime configuration. Callers that
inject a model use `SessionHostBuilder::chat_model_with_config` to bind the same
explicit configuration to staging, provider preparation and acting policy;
builder workspace/action overrides apply to that configuration. Bare
`chat_model` does not silently load operator configuration.

For native PNG input, TinyAgents may provide a smaller losslessly recompressed
derivative. Only IDAT compression/filtering can change: pixel format, hidden RGB
under transparency, interlacing, and every other chunk must remain unchanged.
Animated, malformed, oversized, or unprofitable PNGs retain their original
bytes. Other formats are not recompressed by this helper.

When native input is unavailable:

- Images use the configured vision route, which must itself satisfy model and
  transport capability checks. The bounded readout is presented as untrusted
  attachment content.
- Text-like files contribute bounded decoded text.
- ZIP, TAR, and TAR.GZ contribute a bounded listing with explicit truncation or
  inspection errors. Names are untrusted, and intake never extracts entries.
- Document extraction contributes sections with provenance and truncation
  notices. When PDF rendering is available, at most four selected scanned pages
  receive bounded image readouts.
- Other media and binaries retain metadata and the workspace path. Intake does
  not automatically transcribe audio or perform full video analysis; the agent
  can use terminal tools or delegate further work on the original.

Fallback text can be reused from bounded in-memory and workspace-side caches.
Cache identity includes original content, MIME/path, processing limits, the
document module version, and the configured vision route identity. Transcripts
retain the source references so later requests can reevaluate routing.

## Published document capability

The pinned TinyDocs `0.2.0` release exposes `ExtractDocument`, `InspectImage`
and `RenderPdf`.
Its source tag and all 11 platform archive digests match the published release
manifest. Office extraction, PDF text extraction, and bounded scanned-page
rendering use this native module through the shared bus contract. A disabled or
unavailable module leaves the original workspace path available to tools.

TinyDocs is an in-process native module. Input, section, text, page, pixel, and
output bounds constrain the work exposed by the intake contract, but do not
provide process isolation or a blanket bound on parser-internal allocations.
Caller timeouts bound waiting; dropping a timed-out future does not stop an
already running blocking parser.

## Explicit delegation

Named and collapsed delegation tools accept `image_paths`. Paths resolve against
the acting workspace and pass the same policy checks as other local reads.
Legacy `[IMAGE:path]` markers remain supported; filenames mentioned only in
ordinary prose grant no read. An explicit image selection replaces automatic
parent-image forwarding. A vision task without a resolvable, nonempty image
fails before inference is constructed.

## Further reading

- [Parent module README](../README.md)
- [Agent harness architecture](../../../../../gitbooks/developing/architecture/agent-harness.md)
- [Chat](../../../../../gitbooks/features/chat.md)
