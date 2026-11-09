---
description: Run the TinyComputer browser headless in a Linux container, with a locally built module and unattended automation.
---

# TinyComputer browser on Linux and Docker

The `browser` tool drives Chromium through the TinyComputer module
(`libtinycomputer.so`). The compiled module registry pins TinyComputer
releases only for the platforms they are published for. When none is pinned
for your Linux host, or you need the module built from the exact
`vendor/tinycomputer` gitlink your core was built from, build it yourself and
point the core at it with a module override.

This page covers the container recipe, the loader's directory rules, the
browser settings a headless research agent needs, and how to let cron-driven
turns click and type without an interactive approval.

{% hint style="warning" %}
A local build is the operator's own artifact. Never copy its SHA-256 into
`crates/openhuman-core/src/modules/registry/` as a release pin: pins come
only from a published release.
{% endhint %}

## 1. Build the module in a builder stage

`vendor/tinycomputer/scripts/build-module` does everything the loader needs.
It runs `cargo build --locked --release -p tinycomputer --lib` and installs
`libtinycomputer.so` into `TINYCOMPUTER_MODULE_DIR` (mode `644`, directory
`755`). It writes a `modules.toml` beside the library naming its SHA-256, then
walks every parent directory and fails early if the loader would refuse one.

Initialize the nested submodules first (`git submodule update --init
--recursive vendor/tinycomputer`). TinyComputer vendors its own TinyBus,
agent-browser and agent-desktop. Build from the same checkout as the core:
the host refuses a module whose TinyComputer contract version is incompatible
with its own.

```dockerfile
# Builds libtinycomputer.so from the gitlink this checkout records.
FROM rust:1.99.1-bookworm AS tinycomputer
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential pkg-config ca-certificates \
        libasound2-dev libxdo-dev libxtst-dev libx11-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src/tinycomputer
COPY vendor/tinycomputer/ ./
# Root-owned, 755 all the way up: what the loader accepts.
ENV TINYCOMPUTER_MODULE_DIR=/opt/tinycomputer
RUN scripts/build-module
```

Then add it to the runtime stage of the repository `Dockerfile` (which
already carries the `libasound2`, `libxdo3`, `libxtst6` and `libx11-6`
runtime libraries the module links), together with Chromium:

```dockerfile
RUN apt-get update && apt-get install -y --no-install-recommends \
        chromium fonts-liberation \
    && rm -rf /var/lib/apt/lists/*
# Copy the library and its modules.toml attestation together.
COPY --from=tinycomputer /opt/tinycomputer/ /opt/tinycomputer/
RUN chown -R root:root /opt/tinycomputer \
 && chmod 755 /opt/tinycomputer \
 && chmod 644 /opt/tinycomputer/libtinycomputer.so /opt/tinycomputer/modules.toml
```

### What the loader checks

The TinyBus loader (`vendor/tinybus/crates/tinybus/src/module/host.rs`)
refuses the artifact unless all of these hold:

- **Every directory from `/` down to the module's own** is owned by root or
  by the user the core runs as (uid `10001` in the image). None may be group-
  or world-writable unless it carries the sticky bit. A symlinked path
  component is refused, and the library itself is opened with `O_NOFOLLOW`.
  Do not put the module on a bind mount or named volume: those are usually
  owned by another user or writable by a group.
- **The file** is a regular `.so` under 512 MiB.
- **`modules.toml` beside it**, when present, names the library's exact
  SHA-256. A mismatch is a refusal, so a rebuilt library needs the
  `modules.toml` that `build-module` regenerated. Keep the two together.
  Without that attestation the module still loads but is not an attested
  recipient, so confidential calls such as `StartTask` (the `task` action) are
  not delivered to it.

A refused or faulted module stays refused until the core restarts.

## 2. Load it with a module override

```toml
[modules]
overrides = [{ id = "tinycomputer", path = "/opt/tinycomputer/libtinycomputer.so" }]
```

`id` is the registry id (`tinycomputer`) and `path` an absolute path to the
library. An override is checked before the search path and the release
cache, and bypasses the compiled release digest, which is why only the
operator's own config can name one (`config/schema/modules.rs`,
`modules::ops::local_override`). `[modules] enabled` must stay `true` (the
default).

## 3. Browser settings for a headless research agent

```toml
[browser]
enabled = true
headless = true
chrome_path = "/usr/bin/chromium"
# Keep cookies and logins across sessions and restarts.
profile_mode = "persistent"
profile_path = "/home/openhuman/.openhuman/browser-profile"
download_dir = "/home/openhuman/.openhuman/browser-downloads"
# Opt-in: see "Unattended actions" below.
unattended_actions = ["click", "press"]

[http_request]
# The browser shares this list; "*" is stripped for the browser.
allowed_domains = ["example.com", "docs.example.org"]
```

- `chrome_path` is passed to the module as the browser executable. Debian's
  `chromium` package installs `/usr/bin/chromium`.
- `profile_mode = "persistent"` requires an **absolute** `profile_path`, and
  `download_dir` must be absolute too. The session is refused otherwise. Put
  both under the workspace volume (`/home/openhuman/.openhuman` in the
  image), which the entrypoint chowns to the core's user, so they survive
  container restarts.
- Navigation obeys `[http_request].allowed_domains`. An empty list refuses
  every session. To let the browser go anywhere, set
  `OPENHUMAN_BROWSER_ALLOW_ALL=1` in the container environment instead of
  adding `"*"`.
- agent-browser adds `--no-sandbox` and `--disable-dev-shm-usage` by itself
  when it detects a container, a root user or a small `/dev/shm`.

## 4. Unattended actions

By default, every click, double-click, fill, type, key press, select and
check goes through the forced host approval gate. So does a browser task
paused at `needs_approval`. The forced gate parks only for a live WebChat
turn and denies everything else, so without the setting below a cron job can
open and read pages but never follow a "next page" link.

`[browser] unattended_actions` names the action kinds a **trusted unattended
turn** may take without that approval:

| Name                    | Covers                                                                                                      |
| ----------------------- | ----------------------------------------------------------------------------------------------------------- |
| `click`, `double_click` | Clicking an element, including `find` with `find_action = "click"`                                          |
| `fill`, `type`          | Writing text into a field                                                                                   |
| `press`                 | A key press such as `Enter`                                                                                 |
| `select`, `check`       | Choosing an option, ticking a box                                                                           |
| `task_step`             | A browser `task` paused at `needs_approval` before an irreversible step, approved through `confirm_pending` |

- The tool reads the turn origin from the per-turn `CoreContext` that the
  cron, background-job and workflow entry points scope around their turn.
- **Only** turns whose origin is `TrustedAutomation` with source `Cron`,
  `Background`, or `Workflow { require_approval: false }` qualify.
- WebChat, external channels (Telegram, Slack, …), CLI and direct chat,
  unlabelled turns, and workflows with `require_approval = true` keep the
  forced gate exactly as before, even for a listed action.
- The default is an empty list, which changes nothing.
- An unknown name allows nothing. At load the core logs a warning with the
  number of unknown entries (never their text) and the list of known names.
- Each allowed action is logged at `info` with the action kind, the first 12
  hex characters of its digest and the origin class. Selectors, typed values,
  page content and the job id are never logged. Grep for
  `[browser] unattended action allowed`.
- Payment checkpoints are not `needs_approval` pauses. A task still stops
  there whatever this list says.

`task_step` waves through steps the task controller itself marks
irreversible, such as sending, submitting or deleting. List it only for jobs
whose prompt and allowed websites you control.
