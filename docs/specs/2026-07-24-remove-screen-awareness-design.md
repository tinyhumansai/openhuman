# Remove Screen Awareness Design

## Objective

Remove every shipped screen-awareness and screen-capture capability from
OpenHuman. The desktop app, embedded provider webviews, core RPC/CLI surface,
agent tool surface, and companion must no longer capture, share, inspect, or
reason from the user's screen.

This is a hard removal. The deleted capabilities will not remain behind feature
flags or compatibility stubs.

## Product Boundary

The removal includes:

- the complete Rust `screen_intelligence` domain, including capture sessions,
  vision summaries, screen-derived memory, permission management, screen input
  actions, autocomplete integration, Globe listener controls, RPC controllers,
  CLI commands, and agent tools;
- the `screen_awareness_agent` and every delegation, prompt, registry, MCP
  resource, and tool reference that makes it discoverable;
- the standalone native screenshot tool and local CLI wrappers;
- browser automation actions that return pixels, including `screenshot` and
  computer-use `screen_capture`, across every browser backend;
- the Tauri screen-share source picker, thumbnail capture, command permissions,
  managed state, and injected `getDisplayMedia` shim;
- CEF permission forwarding for desktop audio and desktop video capture, so an
  embedded provider cannot bypass the deleted picker and initiate capture
  directly;
- companion foreground app/window context, screen-capture configuration,
  screen-aware prompting, pointing tags, pointing state, and pointing UI;
- all screen-awareness settings, developer panels, connections/skills tabs,
  setup flows, routes, navigation entries, frontend state, RPC wrappers,
  translations, tool presentation metadata, and product tests; and
- checked-in schemas, capability catalog entries, feature documentation,
  architecture documentation, and test inventories that advertise the removed
  behavior.

The following remain:

- DOM and accessibility-tree browser snapshots that return structured text
  rather than captured pixels;
- user-supplied image attachments, including screenshots the user captured
  outside OpenHuman;
- general image analysis and vision support for those supplied images;
- camera and microphone capture that do not capture a display;
- accessibility helpers still required by voice, dictation, hotkeys, or other
  non-screen features; and
- developer-only test-runner failure screenshots and App Store asset-generation
  scripts, because they are not shipped product APIs.

## Core and Configuration

Delete `src/openhuman/screen_intelligence/` and remove its module registration,
controller schemas, CLI dispatch, legacy aliases, startup/shutdown hooks,
application snapshot data, built-in agents, agent tools, user-tool filters, MCP
prompt resources, and capability catalog entries.

Remove `ScreenIntelligenceConfig` and its update controller from the persisted
configuration schema. Existing user configuration files may still contain a
`screen_intelligence` table after upgrade; serde's normal unknown-field
behavior will ignore it. No migration is required because no retained behavior
consumes the data.

Remove screen capture and Screen Recording permission helpers from the shared
accessibility domain. Retain only independently used accessibility behavior.
Removed RPC names must resolve as unknown methods rather than as deprecated
stubs.

## Browser and Desktop Shell

Remove the top-level `screenshot` tool and its local CLI wrappers. Remove
pixel-producing screenshot variants from browser action parsing, schemas,
native WebDriver execution, Playwright execution, and computer-use sidecar
dispatch. DOM `snapshot` remains the supported inspection action.

Delete the Tauri `screen_capture` module and unregister its managed state and
commands. Remove those commands from application permission allowlists. Delete
the injected screen-share picker and `getDisplayMedia` replacement from the
provider runtime.

Update the vendored CEF permission policy to reject desktop audio and desktop
video capture bits while continuing to allow device microphone and camera
capture. Embedded Meet, Slack, Discord, Zoom, and similar provider webviews will
therefore no longer offer working display sharing through OpenHuman.

## Companion

Keep companion voice and text conversations, but make them screen-independent.
Remove foreground app/window collection, the `capture_screen` and
`include_app_context` configuration fields, screen-context prompt construction,
`[POINT:…]` instructions and parsing, pointing transitions, and corresponding
settings/status UI. The companion receives only the user's current utterance
and conversation history.

## Frontend

Delete the screen-intelligence feature hooks, panels, setup modal, debug
surfaces, and their tests. Remove settings and connections/skills route
registrations, redirects, tab parsing, navigation icons, developer-menu rows,
CoreState fields, RPC method mappings, Tauri command wrappers, translations,
and screen-capture tool labels.

Old hashes such as `/settings/screen-intelligence` and
`/settings/screen-awareness-debug` will fall through to the existing settings
routing behavior. They will not redirect to another screen-awareness surface.

## Compatibility and Security

This change intentionally breaks callers of removed RPC, CLI, Tauri, browser,
and tool interfaces. Retaining callable tombstones would leave a misleading
surface and is outside the goal.

The final CEF policy must deny desktop capture at the permission boundary, not
merely hide the picker UI. Product-wide searches must confirm that no shipped
runtime calls platform screenshot commands, registers display-capture commands,
offers screenshot/browser capture tools, or exposes screen-awareness routes.

## Delivery and Validation

Implement the removal in small dependency-ordered steps, validating and
committing each step separately:

1. Remove the core domain, configuration, lifecycle, app-state, agent, catalog,
   schema, and core tests.
2. Remove native screenshot and browser pixel-capture tools and their tests.
3. Remove Tauri screen sharing, the provider shim, desktop-capture permission
   forwarding, and companion screen behavior.
4. Remove frontend UI, routes, state, wrappers, translations, and tests.
5. Remove or revise documentation, generated schemas, E2E inventories, and
   stale references.

Validation must include Rust formatting, targeted core tests, root
`cargo check`, Tauri tests and `cargo check`, targeted Vitest tests, frontend
typecheck/lint/format checks, schema consistency checks, and repository searches
for residual shipped capture surfaces. Test-only screenshot artifact helpers
and user-image terminology may remain when each surviving match is demonstrably
outside the shipped capture surface.
