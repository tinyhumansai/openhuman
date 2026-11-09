# OSS Scanner setup

This directory maintains OpenHuman's audit environment and threat model for
[Anthropic OSS Scanner](https://github.com/anthropics/oss-scanner).
The enrollment request is
[anthropics/oss-scanner#849](https://github.com/anthropics/oss-scanner/pull/849).
Enrollment and scan scheduling are decided by Anthropic; these files alone do
not mean the service has accepted OpenHuman.

Security findings and build problems go to **security@tinyhumans.ai**, the
address published in [SECURITY.md](../SECURITY.md).

## Enrollment configuration

After these files are on OpenHuman's `main` branch, the scanner enrollment can
use the following `projects/openhuman/project.yaml`:

```yaml
repo: https://github.com/tinyhumansai/openhuman#main
primary_contact: security@tinyhumans.ai
dockerfile: .oss-scanner/Dockerfile
threat_model: .oss-scanner/threat_model.md
```

Remove any `Dockerfile` and `threat_model.md` beside that enrollment file when
switching to these paths: OSS Scanner requires each file in exactly one place.
Until then, the enrollment's existing files remain usable.

## Validate and build

From a checkout of `anthropics/oss-scanner` with that enrollment configuration,
run:

```bash
python3 tools/validate.py
python3 tools/check --no-shell openhuman
python3 tools/check openhuman
```

The final command opens the built image with networking disabled. For example,
run the complete frontend unit suite there with:

```bash
VITEST_MAX_WORKERS=2 VITEST_POOL=threads pnpm test
```

The Dockerfile builds the headless Rust core and React frontend, downloads and
verifies registry-pinned native modules, and runs the complete frontend unit
suite with the project's supported two-worker threads configuration. It keeps
source, build tools and dependency caches for offline investigation. Native
Tauri desktop builds and platform-specific GUI E2E tests are outside this
headless image; their source remains available for review.

For a direct Docker build, use a fresh standalone clone of OpenHuman as the
build context:

```bash
git clone https://github.com/tinyhumansai/openhuman.git openhuman-audit
cd openhuman-audit
docker build -f .oss-scanner/Dockerfile -t openhuman-audit .
```

A linked worktree has `.git` pointers outside the Docker context, so use a
standalone clone for this build. The Dockerfile initializes recursive
submodules itself. Its adjacent `Dockerfile.dockerignore` includes Git
metadata, frontend source, scripts and tests that the production
`.dockerignore` omits, while excluding local environment files and generated build
output.

Keep the Rust image version aligned with `rust-toolchain.toml`, the pnpm version
with `package.json`, and the threat model aligned with current host and module
boundaries. Use local mocks for reproductions that need backend responses;
never supply production credentials or contact production services.
