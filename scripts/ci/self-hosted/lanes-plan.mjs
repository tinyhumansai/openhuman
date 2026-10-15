// The lane plan behind ci-fast.yml / ci-fast-hosted.yml (via ci-lanes.yml).
//
// Pure data: `buildPlan()` turns a profile and the changed-area flags into
// lanes of named checks. `lanes.mjs` executes the plan. Keeping the plan pure
// lets scripts/__tests__/self-hosted-lanes.test.mjs pin its shape without
// running a single cargo command.
//
// Every check here is a check ci-lite.yml runs, moved as-is. Two rules hold for
// all of them:
//
//   * An area flag decides whether a WHOLE suite runs. No command is ever
//     narrowed to changed files: commands are static strings that never
//     interpolate the diff (the self-test asserts it).
//   * Every check in a lane runs even when an earlier one failed (#6451), and
//     the lane still fails. `needs` names only real data dependencies.

/** Changed-area flags, as exported by the workflow from dorny/paths-filter. */
export const AREA_ENV = {
  frontend: "CI_AREA_FRONTEND",
  i18n: "CI_AREA_I18N",
  docs: "CI_AREA_DOCS",
  rustCore: "CI_AREA_RUST_CORE",
  rustTauri: "CI_AREA_RUST_TAURI",
  scripts: "CI_AREA_SCRIPTS",
  inventory: "CI_AREA_INVENTORY",
  toolchainImage: "CI_AREA_TOOLCHAIN_IMAGE",
  installPs1: "CI_AREA_INSTALL_PS1",
  storage: "CI_AREA_STORAGE",
};

/** Read the area flags from an environment object. Missing means false. */
export function areasFromEnv(env) {
  const areas = {};
  for (const [key, name] of Object.entries(AREA_ENV)) {
    areas[key] = env[name] === "true";
  }
  return areas;
}

/** Hosted-runner job groups: lanes that share one GitHub-hosted job. */
export const HOSTED_GROUPS = [
  // Quick checks and the frontend suite fit one 4-core runner side by side.
  {
    group: "checks",
    lanes: ["static", "frontend", "frontend-tests"],
    maxParallel: 3,
    container: true,
  },
  // Lint and gates-off share one `target/` on a ~14 GB disk, so run in turn.
  {
    group: "rust-lint",
    lanes: ["rust-lint", "rust-gates-off"],
    maxParallel: 1,
    container: true,
  },
  { group: "rust-cov", lanes: ["rust-cov"], maxParallel: 1, container: true },
  { group: "tauri", lanes: ["tauri"], maxParallel: 1, container: true },
  // The storage drivers build the core with features no other lane enables,
  // so they get a runner (and a `target/`) of their own.
  {
    group: "storage",
    lanes: ["storage-drivers"],
    maxParallel: 1,
    container: true,
  },
  // pwsh ships on the bare ubuntu-latest image, not in the CI container.
  { group: "pester", lanes: ["pester"], maxParallel: 1, container: false },
];

const CORE_PRODUCT = '"$(bash scripts/ci/product-features.sh --core)"';

// The storage-drivers lane: the drivers it turns on, the root e2e targets that
// run once per driver (tests/support/storage_drivers.rs), and the lib-test
// filters of the domains that sit on the storage ports.
const STORAGE_DRIVERS = "storage-sqlite,storage-file";
const STORAGE_E2E_TARGETS = [
  "storage_approvals_e2e",
  "storage_domains_e2e",
  "storage_flows_e2e",
  "storage_secrets_e2e",
  "storage_scope_e2e",
  "storage_agent_scopes_e2e",
  "storage_delegation_e2e",
  "storage_default_import_e2e",
  "storage_desktop_upgrade_e2e",
  "cli_storage_url_e2e",
];
const STORAGE_LIB_FILTERS = [
  "storage::",
  "cron::",
  "flows::",
  "security::approval::",
  "security::devices::",
  "security::secret_storage::",
  "security::credentials::",
  "desktop::notifications::",
  "integrations::task_sources::",
  "integrations::composio::file_store",
  "agent::orchestration::",
  "config::workspace::",
  "desktop::app_state::",
  "desktop::control::",
  "platform::cost::",
  "inference::tokenjuice::",
  "config::schema::load::source::",
];

/**
 * Build the lane plan.
 *
 * @param {object} opts
 * @param {"ex63"|"hosted"} opts.profile
 * @param {Record<string, boolean>} opts.areas  from areasFromEnv()
 * @param {object} opts.env  process environment (profile paths only)
 * @param {boolean} [opts.isPullRequest]
 * @param {boolean} [opts.tinymoldOnPath]  whether a `tinymold` binary runs
 *   here (lanes.mjs probes it; the plan itself stays pure)
 * @returns {{profile: string, lanes: Lane[], rustWrapper: string|null,
 *   tinymoldLog: string|null}}
 */
export function buildPlan({
  profile,
  areas,
  env = {},
  isPullRequest = true,
  tinymoldOnPath = false,
}) {
  if (profile !== "ex63" && profile !== "hosted") {
    throw new Error(`unknown profile "${profile}" (expected ex63 or hosted)`);
  }
  const ex63 = profile === "ex63";
  const scratch = env.CI_SCRATCH_DIR;
  if (ex63 && !scratch) {
    throw new Error(
      "profile ex63 needs CI_SCRATCH_DIR (set by the microVM guest)",
    );
  }
  const rust = areas.rustCore || areas.rustTauri;
  const gateSmoke = rust || areas.inventory;
  // ex63: the slot's persistent /cache disk keeps the frontend tools' caches
  // (tsc build info, eslint and prettier caches) from job to job. All three
  // key on file content, so a stale entry can only cost a re-check.
  const nodeCache = ex63 ? "${CI_CACHE_DIR:-/cache}/node-tools" : null;
  const core = areas.rustCore;

  // ex63: one throwaway target dir per lane so lanes never queue on cargo's
  // build-dir lock, warmed by a compile cache (rustWrapperFor): tinymold over
  // the slot's /cache disk and the host's shared remote when the guest has
  // both, sccache otherwise. sccache keys include the target dir, so under
  // sccache a lane shares with the same lane of every earlier job on any VM,
  // not with the other lanes.
  // hosted: cargo's default target dirs, which Swatinem/rust-cache restores.
  const targetDir = (lane) => (ex63 ? `${scratch}/target/${lane}` : null);
  const rustWrapper = rustWrapperFor({ profile, env, tinymoldOnPath });
  // tinymold appends one line per compile to TINYMOLD_LOG (a single write per
  // line, so the parallel lanes can share one file); lanes.mjs reads it back
  // with `tinymold stats` for the job summary.
  const tinymoldLog =
    rustWrapper === "tinymold" ? `${scratch}/tinymold.log` : null;
  const wrapperEnv =
    rustWrapper === "tinymold"
      ? { RUSTC_WRAPPER: "tinymold", TINYMOLD_LOG: tinymoldLog }
      : rustWrapper
        ? { RUSTC_WRAPPER: rustWrapper }
        : {};
  const rustEnv = {
    CARGO_INCREMENTAL: "0",
    RUSTFLAGS: "-C link-arg=-fuse-ld=mold",
  };
  // Instrumented builds: no DWARF, a large test stack, and on hosted the
  // serialized build ci-lite needs to fit the runner's disk. On ex63 they go
  // through the compile cache too: cargo-llvm-cov chains an existing
  // RUSTC_WRAPPER in both its wrapper and RUSTFLAGS modes, and cache hits
  // give byte-identical lcov (checked with cargo-llvm-cov 0.8 and sccache
  // 0.10; tinymold keys instrumented compiles on the real checkout path, so
  // their coverage mappings always point at this checkout). Hosted keeps
  // ci-lite's wrapper-free setup.
  const covEnv = {
    ...rustEnv,
    CARGO_PROFILE_DEV_DEBUG: "0",
    RUST_MIN_STACK: "67108864",
    ...(ex63 ? wrapperEnv : { CARGO_BUILD_JOBS: "1" }),
  };
  const modulesDir = ex63
    ? `${scratch}/test-modules`
    : "/opt/openhuman-test-modules/${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}";
  const modulesEnvFile = "ci-out/test-modules.env";
  const withModules = (cmd) =>
    `set -a && . ${modulesEnvFile} && set +a && ${cmd}`;

  // Not run on pull requests: the core doctests (an uninstrumented core build
  // of their own) and the TinyJuice host-module regression (one test against
  // the downloaded module). CI Lite runs both on every push to `main` that
  // touches the Rust core (rust-coverage.sh and its rust-core-coverage job).
  // openhuman-tui's tests DO run on pull requests: the rust-core path filter
  // arms this job for any change under core, embed, tinyhumans, rpc, cli or
  // tui, which is every crate the TUI depends on.
  //
  // Also left to those pushes, as duplicates of what runs here:
  //  - `cargo test -p openhuman-embed` / `-p openhuman-tinyhumans` (default
  //    features): rust-core-coverage already runs both crates' tests, with the
  //    product features;
  //  - `cargo check -p openhuman --no-default-features`: embed-check-no-default
  //    builds the core with exactly that feature set (none) as its dependency.

  /** @type {Lane[]} */
  const lanes = [
    {
      name: "static",
      checks: [
        {
          name: "rust-fmt",
          when: rust,
          run: "cargo fmt --manifest-path Cargo.toml --package openhuman --package openhuman-cli --package openhuman-embed --package openhuman-modules-linked --package openhuman-rpc --package openhuman-tinyhumans --package openhuman-tui -- --check",
        },
        {
          name: "rust-layout",
          when: core,
          run: "node scripts/ci/check-openhuman-rust-layout.mjs",
        },
        {
          name: "agent-runtime-boundary",
          when: core,
          run: "pnpm agent:runtime-boundary",
        },
        {
          name: "saas-ambient",
          when: core,
          run: "pnpm saas:ambient",
        },
        {
          name: "ignored-tests-ratchet",
          when: rust,
          run: "pnpm rust:ignored-tests",
        },
        {
          name: "linux-tls-policy",
          when: rust,
          run: "bash scripts/check-linux-tls-dependencies.sh",
        },
        {
          name: "gated-test-allowlist",
          when: rust,
          run: "bash scripts/ci/check-gated-test-allowlist.sh",
        },
        {
          name: "orch-ip-gate",
          when: true,
          run: "bash scripts/ci/orch-ip-gate.sh",
        },
        {
          name: "feature-forwarding",
          when: true,
          run: "node scripts/ci/check-feature-forwarding.mjs",
        },
        {
          // core -> embed -> tinyhumans -> rpc -> app/cli/tui; cheap, and a
          // manifest edit anywhere can break it, so always on.
          name: "crate-chain",
          when: true,
          run: "node scripts/ci/check-crate-chain.mjs",
        },
        {
          name: "process-statics",
          when: true,
          run: "bash scripts/ci-cancel-aware.sh node scripts/ci/check-process-statics.mjs",
        },
        {
          // CI runs the pinned release only; linked/source are local-dev modes.
          name: "module-source",
          when: true,
          run: "node scripts/ci/check-ci-module-source.mjs",
        },
        {
          name: "module-boundaries",
          when: rust || areas.scripts,
          run: "bash scripts/ci-cancel-aware.sh node scripts/ci/check-module-boundaries.mjs",
        },
        {
          name: "module-pins",
          when: true,
          run: "bash scripts/ci/fetch-submodule-tags.sh && node scripts/ci/check-module-pins.mjs",
        },
        {
          name: "submodule-monotonic",
          when: isPullRequest,
          needs: ["module-pins"],
          run: "node scripts/ci/check-submodule-monotonic.mjs",
        },
        {
          name: "toolchain-image-drift",
          when: areas.toolchainImage,
          run: "node scripts/ci/check-toolchain-image.mjs",
        },
        {
          name: "test-inventory",
          when: areas.inventory,
          run: "pnpm test:inventory",
        },
      ],
    },
    {
      name: "frontend",
      env: { NODE_ENV: "test" },
      checks: [
        {
          name: "pnpm-install",
          // All Rust changes run the bus-only dependency guard, whose TOML
          // parser is a root package dependency. ex63 shares this install
          // with rust-core-coverage's mock backend; hosted lanes are separate.
          when: areas.frontend || areas.i18n || areas.scripts || rust,
          run: "pnpm install --frozen-lockfile",
        },
        {
          name: "tsc",
          when: areas.frontend,
          needs: ["pnpm-install"],
          run: ex63
            ? `mkdir -p ${nodeCache}/tsc && pnpm --filter openhuman-app compile --tsBuildInfoFile ${nodeCache}/tsc/app.tsbuildinfo`
            : "pnpm --filter openhuman-app compile",
        },
        {
          name: "prettier",
          when: areas.frontend,
          needs: ["pnpm-install"],
          // ex63: ci-lite's `format:check` is prettier plus rust:format:check;
          // the static lane's rust-fmt already checks OpenHuman workspace packages, so
          // only the (non-member) Tauri crate's rustfmt check stays here.
          run: ex63
            ? `pnpm --filter openhuman-app format:check:prettier --cache --cache-strategy content --cache-location ${nodeCache}/prettier-cache` +
              " && cargo fmt --manifest-path crates/openhuman-app/Cargo.toml --package openhuman-app -- --check"
            : "pnpm --filter openhuman-app format:check",
        },
        {
          name: "eslint",
          when: areas.frontend,
          needs: ["pnpm-install"],
          // `lint` already passes --cache; these point it at the persistent
          // disk and key it on content (a fresh checkout resets every mtime).
          run: ex63
            ? `mkdir -p ${nodeCache}/eslint && pnpm --filter openhuman-app lint --cache-strategy content --cache-location ${nodeCache}/eslint/`
            : "pnpm --filter openhuman-app lint",
        },
        {
          name: "i18n",
          when: areas.i18n,
          needs: ["pnpm-install"],
          run: "pnpm i18n:check",
        },
        {
          name: "docs-generator-tests",
          when: areas.docs,
          run: "pnpm docs:test",
        },
        { name: "docs-drift", when: areas.docs, run: "pnpm docs:check" },
        {
          name: "security-module-dependencies",
          when: areas.scripts || rust,
          needs: ["pnpm-install"],
          run: "node scripts/ci/check-security-module-dependencies.mjs",
        },
        {
          name: "scripts-self-tests",
          when: areas.scripts,
          needs: ["pnpm-install"],
          run: "pnpm test:scripts",
        },
      ],
    },
    {
      // Split from `frontend` so the long vitest run overlaps the lint checks.
      name: "frontend-tests",
      env: {
        NODE_ENV: "test",
        VITEST_MAX_WORKERS: ex63 ? "8" : "3",
        ...(ex63 ? { VITEST_POOL: "threads" } : {}),
      },
      checks: [
        {
          name: "vitest-coverage",
          when: areas.frontend,
          needs: ["frontend:pnpm-install"],
          run:
            "pnpm --filter openhuman-app test:coverage" +
            " && test -f app/coverage/lcov.info" +
            " && sed -E -e 's#^SF:(src/)#SF:app/\\1#' -e 's#^SF:(\\./src/)#SF:app/src/#'" +
            " app/coverage/lcov.info > ci-out/lcov/lcov-frontend.info",
        },
      ],
    },
    {
      // First among the Rust lanes: it is the long pole, and rust-lint waits
      // on its test-modules check.
      name: "rust-cov",
      // Compiles the core crate: holds one of the VM's heavy-compile slots
      // (lanes.mjs). Lower number = served first.
      heavy: 0,
      targetDir: targetDir("cov"),
      env: covEnv,
      checks: [
        // hosted: rust-cov has a runner of its own, so it installs the node
        // deps the mock backend needs itself (ex63 reuses frontend's install).
        {
          name: "pnpm-install",
          when: !ex63 && core,
          run: "pnpm install --frozen-lockfile",
        },
        {
          name: "test-modules",
          when: core,
          run: `bash scripts/ci/self-hosted/install-test-modules.sh "${modulesDir}" ${modulesEnvFile}`,
        },
        {
          name: "rust-core-coverage",
          when: core,
          needs: [
            "test-modules",
            ex63 ? "frontend:pnpm-install" : "pnpm-install",
          ],
          // Doctests run on pushes to main instead (see above); the tui suite
          // runs here (OH_COV_TUI=1).
          // ex63: the core's unit tests run under cargo-nextest, one process
          // per test and in parallel (the guest image ships cargo-nextest).
          env: {
            OUT: "ci-out/lcov/lcov-core.info",
            OH_COV_DOCTESTS: "0",
            OH_COV_TUI: "1",
            ...(ex63 ? { OH_COV_RUNNER: "nextest" } : {}),
          },
          run: withModules("bash scripts/ci/rust-coverage.sh"),
        },
      ],
    },
    {
      name: "rust-lint",
      // Compiles the core crate: holds one of the VM's heavy-compile slots
      // (lanes.mjs). Lower number = served first.
      heavy: 1,
      targetDir: targetDir("lint"),
      env: { ...rustEnv, ...wrapperEnv },
      checks: [
        // `--features` is load-bearing: `default` is the contributor set.
        {
          name: "clippy-product",
          when: core,
          run: `cargo clippy -p openhuman --features ${CORE_PRODUCT} -- -D warnings`,
        },
        // No separate `cargo clippy -p openhuman` (contributor set): clippy
        // lints every workspace crate in the graph, and openhuman-embed's
        // default features are exactly the core's, so embed-clippy already
        // lints the core's library with them under -D warnings.
        {
          name: "embed-clippy",
          when: core,
          run: "cargo clippy -p openhuman-embed --all-targets -- -D warnings",
        },
        {
          name: "embed-check-no-default",
          when: core,
          run: "cargo check -p openhuman-embed --no-default-features",
        },
        {
          name: "embed-doctests",
          when: core,
          run: "bash scripts/ci-cancel-aware.sh cargo test -p openhuman-embed --doc",
        },
        {
          name: "embed-rustdoc",
          when: core,
          env: { RUSTDOCFLAGS: "-D warnings" },
          run: "bash scripts/ci-cancel-aware.sh cargo doc -p openhuman-embed --no-deps",
        },
        {
          name: "embed-offline-examples",
          when: core,
          run: "node scripts/run-embed-examples.mjs",
        },
        {
          name: "tinyhumans-clippy",
          when: core,
          run: "cargo clippy -p openhuman-tinyhumans --all-targets -- -D warnings",
        },
      ],
    },
    {
      name: "rust-gates-off",
      // Compiles the core crate: holds one of the VM's heavy-compile slots
      // (lanes.mjs). Lower number = served first.
      heavy: 1,
      targetDir: targetDir("gatesoff"),
      env: { ...rustEnv, ...wrapperEnv, RUST_MIN_STACK: "67108864" },
      checks: [
        {
          name: "gate-contract-tests",
          when: gateSmoke,
          run:
            "cargo test --manifest-path Cargo.toml -p openhuman --no-default-features --lib --" +
            " core::all:: core::cli:: core::invoke:: core::session_expiry:: core::legacy_aliases:: core::runtime::" +
            " agent::registry::agents::loader:: commands::ops::tests:: memory::people::contacts_gate_tests::" +
            " memory::layout_migration::" +
            " openhuman::config:: openhuman::platform::socket::event_handlers:: tools::schemas:: tools::ops::tests::",
        },
        {
          // Share one build for these disjoint test surfaces. Enable the
          // module host while keeping web3 disabled, so its wallet callback
          // gate is exercised rather than silently selecting zero tests.
          // MCP resources do not touch test_support; introspection stays
          // scoped to avoid tests that require the complete product. The
          // wallet grant's ungated tests sit beside a web3-gated one, so they
          // run here with web3 off (check-gated-test-allowlist.sh).
          name: "gate-contract-tests-features",
          when: gateSmoke,
          run:
            "cargo test --manifest-path Cargo.toml -p openhuman --no-default-features" +
            " --features modules,mcp,e2e-test-support --lib --" +
            " modules::runtime:: modules::registry:: modules::storage_grant_tinywallet::" +
            " modules::storage_grant_tinyconnectors::" +
            " mcp::server::resources:: test_support::introspect::",
        },
        {
          name: "kernel-floor",
          when: gateSmoke,
          run: "bash scripts/check-kernel-floor.sh --verbose",
        },
        {
          name: "dep-sim-calibration",
          when: gateSmoke,
          run: "bash scripts/ci/check-dep-sim-calibration.sh",
        },
      ],
    },
    {
      name: "tauri",
      // Compiles the core crate: holds one of the VM's heavy-compile slots
      // (lanes.mjs). Lower number = served first.
      heavy: 2,
      targetDir: targetDir("tauri"),
      env: { ...rustEnv },
      checks: [
        {
          name: "clippy-tauri",
          when: areas.rustTauri,
          env: wrapperEnv,
          run: "cargo clippy --manifest-path crates/openhuman-app/Cargo.toml -- -D warnings",
        },
        {
          // llvm-cov's RUSTFLAGS mode with the linker flag cleared, as in
          // ci-lite. ci-lite also clears RUSTC_WRAPPER because its container
          // config installs sccache; on ex63 covEnv sets the wrapper on purpose.
          name: "tauri-coverage",
          when: areas.rustTauri,
          env: { ...covEnv },
          run:
            (ex63 ? "unset RUSTFLAGS" : "unset RUSTFLAGS RUSTC_WRAPPER") +
            " && cargo llvm-cov clean --manifest-path crates/openhuman-app/Cargo.toml" +
            " && cargo llvm-cov --no-rustc-wrapper --manifest-path crates/openhuman-app/Cargo.toml" +
            " --lcov --output-path ci-out/lcov/lcov-tauri.info",
        },
      ],
    },
    {
      // The sqlite and file storage drivers, which no other lane compiles in:
      // every `storage_*_e2e` target runs once per driver (memory, sqlite,
      // file) and the stores that sit on the storage ports run their lib
      // tests with the drivers on. The MongoDB driver needs a replica set and
      // has its own workflow (storage-mongodb.yml). Armed by the `storage`
      // area (.github/ci-paths-filter.yml).
      name: "storage-drivers",
      // Compiles the core crate with its own feature set: holds one of the
      // VM's heavy-compile slots (lanes.mjs). Lower number = served first.
      heavy: 2,
      targetDir: targetDir("storage"),
      env: { ...rustEnv, ...wrapperEnv, RUST_MIN_STACK: "16777216" },
      checks: [
        {
          name: "storage-e2e",
          when: areas.storage,
          run:
            `cargo test --no-fail-fast -p openhuman-cli --features ${STORAGE_DRIVERS}` +
            STORAGE_E2E_TARGETS.map((t) => ` --test ${t}`).join(""),
        },
        {
          name: "storage-lib-tests",
          when: areas.storage,
          run:
            `cargo test -p openhuman --features ${STORAGE_DRIVERS} --lib --` +
            STORAGE_LIB_FILTERS.map((f) => ` ${f}`).join(""),
        },
        {
          name: "storage-session-store-tests",
          when: areas.storage,
          run: `cargo test -p openhuman-rpc --features session-store,${STORAGE_DRIVERS} --lib -- session_store::`,
        },
      ],
    },
    {
      name: "pester",
      checks: [
        {
          name: "install-ps1-pester",
          when: areas.installPs1,
          run: "pwsh -NoProfile -File scripts/tests/OpenHumanWindowsInstall.Tests.ps1",
        },
      ],
    },
  ];

  for (const lane of lanes) {
    lane.checks = lane.checks.map((c) => ({
      ...c,
      when: Boolean(c.when),
      needs: c.needs ?? [],
    }));
    lane.active = lane.checks.some((c) => c.when);
  }
  return { profile, lanes, rustWrapper, tinymoldLog };
}

/**
 * The RUSTC_WRAPPER the Rust lanes compile through.
 *
 * - hosted: none (ci-lite's wrapper-free setup; rust-cache restores target/).
 * - ex63 with a `tinymold` binary and TINYMOLD_REMOTE_URL: tinymold. The
 *   microVM guest (tinyhumansai/gh-hosted-runner) exports the TINYMOLD_REMOTE_*
 *   variables only while the host's shared cache answers, and TINYMOLD_STORE
 *   points at the slot's persistent /cache disk.
 * - ex63 otherwise: sccache, exactly as before tinymold.
 *
 * @param {object} opts
 * @param {"ex63"|"hosted"} opts.profile
 * @param {object} [opts.env]
 * @param {boolean} [opts.tinymoldOnPath]
 * @returns {"tinymold"|"sccache"|null}
 */
export function rustWrapperFor({ profile, env = {}, tinymoldOnPath = false }) {
  if (profile !== "ex63") return null;
  return tinymoldOnPath && env.TINYMOLD_REMOTE_URL ? "tinymold" : "sccache";
}

/** Restrict a plan to the named lanes (hosted groups run a subset each). */
export function selectLanes(plan, names) {
  if (!names || names.length === 0) return plan;
  const known = new Set(plan.lanes.map((l) => l.name));
  for (const n of names) {
    if (!known.has(n)) throw new Error(`unknown lane "${n}"`);
  }
  return { ...plan, lanes: plan.lanes.filter((l) => names.includes(l.name)) };
}

/**
 * Hosted matrix: one entry per group with at least one active lane, so an
 * untouched area never spins up a runner.
 */
export function hostedMatrix(plan) {
  const active = new Set(plan.lanes.filter((l) => l.active).map((l) => l.name));
  return HOSTED_GROUPS.filter((g) => g.lanes.some((l) => active.has(l))).map(
    (g) => ({
      group: g.group,
      lanes: g.lanes.join(","),
      "max-parallel": g.maxParallel,
      container: g.container ? "ghcr.io/tinyhumansai/openhuman_ci:latest" : "",
    }),
  );
}

/**
 * Check every cross-lane and in-lane `needs` resolves, and that no dependency
 * cycle exists. Returns a list of problems (empty when the plan is sound).
 */
export function validatePlan(plan) {
  const problems = [];
  const ids = new Set();
  for (const lane of plan.lanes) {
    for (const c of lane.checks) ids.add(`${lane.name}:${c.name}`);
  }
  for (const lane of plan.lanes) {
    for (const c of lane.checks) {
      for (const need of c.needs) {
        const id = need.includes(":") ? need : `${lane.name}:${need}`;
        if (!ids.has(id))
          problems.push(`${lane.name}:${c.name} needs unknown check ${id}`);
      }
    }
  }
  return problems;
}

/**
 * @typedef {object} Check
 * @property {string} name
 * @property {boolean} when      run it at all (area-selected)
 * @property {string} run        static bash command
 * @property {string[]} needs    checks (`name` or `lane:name`) that must succeed first
 * @property {object} [env]
 * @property {boolean} [reportOnly]  failure never fails the lane
 *
 * @typedef {object} Lane
 * @property {string} name
 * @property {Check[]} checks
 * @property {string|null} [targetDir]  CARGO_TARGET_DIR for this lane
 * @property {object} [env]
 * @property {number} [nice]
 * @property {number} [heavy]  compiles the core crate; priority for a
 *   heavy-compile slot (lower first). Absent for light lanes.
 * @property {boolean} active
 */
