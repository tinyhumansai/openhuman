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

  // pwsh ships on the bare ubuntu-latest image, not in the CI container.
  { group: "pester", lanes: ["pester"], maxParallel: 1, container: false },
];

const CORE_PRODUCT = '"$(bash scripts/ci/product-features.sh --core)"';


/**
 * Build the lane plan.
 *
 * @param {object} opts
 * @param {"ex63"|"hosted"} opts.profile
 * @param {Record<string, boolean>} opts.areas  from areasFromEnv()
 * @param {object} opts.env  process environment (profile paths only)
 * @param {boolean} [opts.isPullRequest]
 * @returns {{profile: string, lanes: Lane[]}}
 */
export function buildPlan({ profile, areas, env = {}, isPullRequest = true }) {
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

  // Private lanes run sequentially and share compatible build products.
  // Instrumented objects stay separate from ordinary build/link checks.
  const targetDir = (lane) =>
    ex63 ? `target/ci-private-${lane === "cov" ? "coverage" : "build"}` : null;
  const rustEnv = ex63
    ? {
        CARGO_INCREMENTAL: "0",
        // Keep file/line backtraces without full variable/type DWARF in
        // ordinary dev and inherited test builds. Coverage overrides this.
        CARGO_PROFILE_DEV_DEBUG: "line-tables-only",
        CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER:
          "/usr/local/bin/rust-mold-linker",
      }
    : {
        CARGO_INCREMENTAL: "0",
        RUSTFLAGS: "-C link-arg=-fuse-ld=mold",
      };
  const covEnv = {
    ...rustEnv,
    CARGO_PROFILE_DEV_DEBUG: "0",
    RUST_MIN_STACK: "67108864",
    // Nextest forks per test. LLVM merges counters losslessly into eight
    // files per binary instead of retaining a separate file per process.
    ...(ex63 ? { LLVM_PROFILE_FILE_NAME: "openhuman-%8m.profraw" } : {}),
    ...(!ex63 ? { CARGO_BUILD_JOBS: "1" } : {}),
  };
  const modulesDir = ex63
    ? `${scratch}/test-modules`
    : "/tmp/openhuman-test-modules-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}";
  const modulesEnvFile = "ci-out/test-modules.env";
  const withModules = (cmd) =>
    `set -a && . ${modulesEnvFile} && set +a && ${cmd}`;

  // Not run on pull requests: the core doctests (an uninstrumented core build
  // of their own) and the TinyJuice host-module regression (one test against
  // the downloaded module). CI Lite, which ran both on pushes to `main`, is
  // gone; today only the release pretest runs them (test-reusable.yml).
  // openhuman-tui's tests DO run on pull requests: the rust-core path filter
  // arms this job for any change under core, embed, tinyhumans, rpc, cli or
  // tui, which is every crate the TUI depends on.
  //
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
          // tinystoragedrivers owns every storage backend; reads cargo
          // metadata for the root workspace and the app crate.
          name: "storage-owner",
          when: rust,
          run: "node scripts/ci/check-storage-owner.mjs",
        },
        {
          // The composed storage schema: names, casing, single ownership and
          // the generated docs/storage-schema.md.
          name: "storage-schema",
          when: rust,
          // ex63: build the dump example in the plain build target with that
          // build's rustEnv and the core product features, which
          // composio-stack-regression-uninstrumented also builds the cli with,
          // so the core is not compiled again here (default target,
          // incremental, full debuginfo). The schema depends only on gates
          // that are in both feature sets, so the dump is unchanged.
          ...(ex63
            ? {
                env: {
                  ...rustEnv,
                  CARGO_BUILD_TARGET_DIR: targetDir("build"),
                  STORAGE_SCHEMA_FEATURES: "core-product",
                },
              }
            : {}),
          run: "node scripts/ci/check-storage-schema.mjs",
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
          name: "storage-bypass",
          when: true,
          run: "bash scripts/ci-cancel-aware.sh node scripts/ci/check-storage-bypass.mjs",
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
      env: {
        NODE_ENV: "test",
        // ESLint exhausts Node's default ~2 GiB heap on the private guest.
        // Lanes are sequential, so this budget does not overlap Rust builds.
        ...(ex63 ? { NODE_OPTIONS: "--max-old-space-size=4096" } : {}),
      },
      checks: [
        {
          name: "pnpm-install",
          // ex63: also whenever rust-core-coverage runs (a core or storage
          // change), so its mock backend (scripts/mock-api-server.mjs imports
          // `ws`) can start when no frontend file changed. One install per VM:
          // lanes share a checkout.
          when:
            areas.frontend ||
            areas.i18n ||
            areas.scripts ||
            (ex63 && (core || areas.storage)),
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
          name: "browser-test-skip-guard",
          when: areas.frontend,
          needs: ["pnpm-install"],
          run: "node scripts/ci/check-browser-test-skips.mjs",
        },
        {
          name: "e2e-runner-ownership",
          when: areas.frontend,
          needs: ["pnpm-install"],
          run: "pnpm e2e:ownership:check",
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
        {
          name: "docs-drift",
          when: areas.docs,
          // ex63: `docs:check` runs the storage schema dump and the embed
          // capability report through cargo. Without the shared build target
          // and rustEnv they compiled the core again in the plain target,
          // incremental and with full debuginfo, so none of the shared build
          // applied (315 s in EX63 run 39371566398). With them the dump is
          // the storage-schema check's build and the report the embed
          // examples'.
          ...(ex63
            ? {
                env: {
                  ...rustEnv,
                  CARGO_BUILD_TARGET_DIR: targetDir("build"),
                  STORAGE_SCHEMA_FEATURES: "core-product",
                },
              }
            : {}),
          run: "pnpm docs:check",
        },
        {
          name: "scripts-self-tests",
          when: areas.scripts,
          needs: ["pnpm-install"],
          run: "pnpm test:scripts && pnpm e2e:ownership:check && node scripts/ci/check-browser-test-skips.mjs",
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
      // Compiles the core crate; hosted runners can cap heavy-compile slots
      // (lanes.mjs). Lower number = served first.
      heavy: 0,
      targetDir: targetDir("cov"),
      env: covEnv,
      checks: [
        // hosted: rust-cov has a runner of its own, so it installs the node
        // deps the mock backend needs itself (ex63 reuses frontend's install).
        {
          name: "pnpm-install",
          when: !ex63 && (core || areas.storage),
          run: "pnpm install --frozen-lockfile",
        },
        {
          name: "test-modules",
          when: core || areas.storage,
          run: `bash scripts/ci/self-hosted/install-test-modules.sh "${modulesDir}" ${modulesEnvFile}`,
        },
        {
          name: "rust-core-coverage",
          when: core || areas.storage,
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
        // ex63: lcov-core.info is written (even after a failed suite), so drop
        // its ~90 GB of instrumented test executables and profiles before the
        // next lanes build on the same 320 GB disk. No `needs`: it must run
        // after a failure too.
        {
          name: "prune-coverage-binaries",
          when: ex63 && (core || areas.storage),
          run: "bash scripts/ci/self-hosted/prune-coverage-binaries.sh target/ci-private-coverage",
        },
      ],
    },
    {
      name: "rust-lint",
      // Compiles the core crate; hosted runners can cap heavy-compile slots
      // (lanes.mjs). Lower number = served first.
      heavy: 1,
      targetDir: targetDir("lint"),
      env: { ...rustEnv },
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
        // Hosts such as Chottu run modules with neither legacy storage import
        // nor the encrypted secrets file; both are gated separately.
        {
          name: "embed-check-no-default-modules",
          when: core,
          run: "bash scripts/ci-cancel-aware.sh cargo check -p openhuman-embed --no-default-features --features modules",
        },
        // Consumers such as TinySweeper enable tracing without native modules.
        {
          name: "embed-check-no-default-langfuse",
          when: core,
          run: "bash scripts/ci-cancel-aware.sh cargo check -p openhuman-embed --no-default-features --features langfuse",
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
          name: "install-test-modules",
          when: core,
          run: `bash scripts/ci/self-hosted/install-test-modules.sh "${modulesDir}" ${modulesEnvFile}`,
        },
        {
          name: "embed-offline-examples",
          when: core,
          needs: ["install-test-modules"],
          env: { RUST_MIN_STACK: "67108864" },
          // Dev profile: the release-declared linux_fleet measurement then
          // shares the plain dev build instead of a thin-LTO one nothing else
          // can reuse (and that would need cleaning afterwards).
          run: withModules("node scripts/run-embed-examples.mjs --profile dev"),
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
      // Compiles the core crate; hosted runners can cap heavy-compile slots
      // (lanes.mjs). Lower number = served first.
      heavy: 1,
      targetDir: targetDir("gatesoff"),
      env: { ...rustEnv, RUST_MIN_STACK: "67108864" },
      checks: [
        {
          name: "gate-contract-tests",
          when: gateSmoke,
          run:
            "cargo test --manifest-path Cargo.toml -p openhuman --no-default-features" +
            " --features storage-sqlite --lib --" +
            " core::all:: core::cli:: core::invoke:: core::session_expiry:: core::legacy_aliases:: core::runtime::" +
            " agent_registry::agents::loader:: commands::ops::tests:: memory::people::contacts_gate_tests::" +
            " memory::layout_migration::" +
            " doctor::checks::" +
            " core::schema_test_support::" +
            " openhuman::config:: openhuman::platform::socket::event_handlers:: tools::schemas:: tools::ops::tests::",
        },
        {
          // Share one build for these disjoint test surfaces. Enable the
          // module host while keeping web3 disabled, so its wallet callback
          // gate is exercised rather than silently selecting zero tests.
          // storage-sqlite supplies the test-only persistence fixtures needed
          // to compile the core test crate when the product defaults are off.
          // MCP resources do not touch test_support; introspection stays
          // scoped to avoid tests that require the complete product. The
          // wallet grant's ungated tests sit beside a web3-gated one, so they
          // run here with web3 off (check-gated-test-allowlist.sh).
          name: "gate-contract-tests-features",
          when: gateSmoke,
          run:
            "cargo test --manifest-path Cargo.toml -p openhuman --no-default-features" +
            " --features modules,mcp,e2e-test-support,storage-sqlite --lib --" +
            " modules::runtime:: modules::registry:: modules::storage_grant_tinywallet::" +
            " modules::storage_grant_tinyconnectors:: modules::tinyflows::" +
            " doctor::checks::" +
            " cron::bus_host::tests:: cron::documents_port::tests::" +
            " mcp::server::resources:: test_support::introspect:: --test-threads=1",
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
      // Compiles the core crate; hosted runners can cap heavy-compile slots
      // (lanes.mjs). Lower number = served first.
      heavy: 2,
      targetDir: targetDir("tauri"),
      env: { ...rustEnv },
      checks: [
        {
          name: "clippy-tauri",
          when: areas.rustTauri,
          env: {},
          run: "cargo clippy --manifest-path crates/openhuman-app/Cargo.toml -- -D warnings",
        },
        {
          // Hosted keeps ci-lite's wrapper-free RUSTFLAGS instrumentation.
          // The private plan below uses the Cargo target linker and nextest.
          name: "tauri-coverage",
          when: areas.rustTauri,
          env: { ...covEnv },
          run:
            "unset RUSTFLAGS RUSTC_WRAPPER" +
            " && cargo llvm-cov clean --manifest-path crates/openhuman-app/Cargo.toml" +
            " && cargo llvm-cov --no-rustc-wrapper --manifest-path crates/openhuman-app/Cargo.toml" +
            " --lcov --output-path ci-out/lcov/lcov-tauri.info",
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

  // Coverage replaces each existing plain suite only for the private profile.
  // Preserve the original package, features, target list, and libtest substring
  // filters; union those filters in nextest's expression syntax. --no-report
  // retains prior instrumentation implicitly. A suite writes no report of its
  // own: every `cargo llvm-cov report` re-merges ALL profiles in the shared
  // coverage target (rust-cov's included), so each lane ends with one report
  // check per workspace instead of one per suite. A report check has no
  // `needs`, so it still reports after a failed suite; the suite keeps its
  // own failure status.
  // Do not also pass --no-clean: the pinned cargo-llvm-cov rejects that pair.
  if (ex63) {
    // This target has cfg(not(coverage)): instrumentation makes its large
    // binary fail to link and would remove its deliberately small-stack test.
    // Run that distinct regression once in the ordinary build target, keeping
    // the CLI's defaults plus the same product features as core coverage.
    lanes
      .find((lane) => lane.name === "rust-lint")
      .checks.push({
        name: "composio-stack-regression-uninstrumented",
        when: core,
        run: `bash scripts/ci/nextest-coverage.sh composio-stack-uninstrumented cargo nextest run --profile ci --no-tests=fail --no-fail-fast -p openhuman-cli --features ${CORE_PRODUCT} --test composio_list_tools_stack_overflow_regression`,
      });
    const coverageSuite = (args, suite) =>
      `bash scripts/ci/nextest-coverage.sh ${suite} cargo llvm-cov nextest --profile ci --no-fail-fast --no-tests=fail --no-report ${args}`;
    const coverageReport = (name, output, when, manifest) => ({
      name,
      when,
      env: { ...covEnv, CARGO_BUILD_TARGET_DIR: "target/ci-private-coverage" },
      run:
        "unset RUSTFLAGS; " +
        `cargo llvm-cov report --manifest-path ${manifest} --lcov --output-path ci-out/lcov/${output}`,
    });
    for (const lane of lanes) {
      if (lane.name !== "rust-gates-off") continue;
      for (const check of lane.checks) {
        if (!check.run.startsWith("cargo test ")) continue;
        const [cargoArgs, filters] = check.run
          .slice("cargo test ".length)
          .split(" -- ");
        const args = cargoArgs.replace("--no-fail-fast ", "");
        let expression = "";
        let serial = "";
        if (filters) {
          const include = [];
          const exclude = [];
          const tokens = filters.trim().split(/\s+/);
          for (let i = 0; i < tokens.length; i++) {
            if (tokens[i] === "--skip") exclude.push(tokens[++i]);
            else if (tokens[i] === "--test-threads=1")
              serial = " --test-threads=1";
            else include.push(tokens[i]);
          }
          const terms = include.map((f) => `test(${f})`).join(" | ");
          expression = ` -E '${terms ? `(${terms})` : "all()"}${exclude.map((f) => ` & not test(${f})`).join("")}'`;
        }
        check.env = {
          ...covEnv,
          CARGO_BUILD_TARGET_DIR: "target/ci-private-coverage",
        };
        check.run = coverageSuite(`${args}${expression}${serial}`, check.name);
      }
      // One merge of the shared profiles for every gates-off suite above.
      lane.checks.push(
        coverageReport(
          "gates-off-coverage-report",
          "lcov-gates-off.info",
          gateSmoke,
          "Cargo.toml",
        ),
      );
    }
    const tauriLane = lanes.find((l) => l.name === "tauri");
    // Clippy emits metadata only; a real uninstrumented link proves that
    // Tauri-only changes also use the target linker checked by the workflow.
    tauriLane.checks.splice(1, 0, {
      name: "build-tauri",
      when: areas.rustTauri,
      run: "bash scripts/ci-cancel-aware.sh cargo build --manifest-path crates/openhuman-app/Cargo.toml --bin OpenHuman",
    });
    const tauri = tauriLane.checks.find((c) => c.name === "tauri-coverage");
    tauri.env = {
      ...covEnv,
      CARGO_BUILD_TARGET_DIR: "target/ci-private-coverage",
    };
    tauri.run =
      "unset RUSTFLAGS; " +
      coverageSuite(
        "--manifest-path crates/openhuman-app/Cargo.toml",
        "tauri", // the nextest report name this suite always had
      );
    // The app is a separate workspace, so its report is a second merge.
    tauriLane.checks.push(
      coverageReport(
        "tauri-coverage-report",
        "lcov-tauri.info",
        areas.rustTauri,
        "crates/openhuman-app/Cargo.toml",
      ),
    );
  }

  for (const lane of lanes) {
    lane.checks = lane.checks.map((c) => ({
      ...c,
      when: Boolean(c.when),
      needs: c.needs ?? [],
    }));
    lane.active = lane.checks.some((c) => c.when);
  }
  return { profile, lanes };
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
 * @property {string|null} [targetDir]  Cargo build.target-dir for this lane
 * @property {object} [env]
 * @property {number} [nice]
 * @property {number} [heavy]  compiles the core crate; priority for a
 *   heavy-compile slot (lower first). Absent for light lanes.
 * @property {boolean} active
 */
