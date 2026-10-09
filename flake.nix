{
  description = "OpenHuman — a Rust-core agent harness with desktop, web, terminal and library front-ends";

  # ---------------------------------------------------------------------------
  # IMPORTANT — submodules
  # ---------------------------------------------------------------------------
  # OpenHuman vendors ~19 `tiny*` crates under `vendor/` as git submodules, and
  # several of them vendor a nested copy of `tinytools` / `tinyinference` /
  # `tinystoragedrivers`. The root Cargo.toml links those by path (`[patch]`
  # tables), so a checkout without submodule contents cannot resolve its
  # dependency graph at all.
  #
  # Nix's flake-source copier deliberately DROPS git submodule contents unless
  # asked for them, so a bare `nix build .#openhuman-core` on a fresh clone
  # fails with "Path 'vendor' ... is not tracked by Git". Use a ref that
  # carries them:
  #
  #   nix develop "git+file://$PWD?submodules=1"
  #   nix build  "git+file://$PWD?submodules=1#openhuman-core"
  #
  # or, when the submodules are already checked out and you want uncommitted
  # edits visible, the `path:` copier takes the working tree verbatim:
  #
  #   nix develop path:.
  #
  # The same `?submodules=1` works for remote refs:
  #
  #   nix run "github:tinyhumansai/openhuman?submodules=1#openhuman-core"
  #
  # A consumer who forgets it gets a cargo "failed to read .../Cargo.toml"
  # rather than a silently wrong build. See NIX.md for the full story.
  # ---------------------------------------------------------------------------

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # rust-toolchain.toml pins Rust 1.96.1 (rusqlite 0.40 / libsqlite3-sys 0.38
    # need the `cfg_select!` macro stabilized in 1.96). rust-overlay honours
    # that file, so the dev shell and the packaged build use exactly the pinned
    # toolchain instead of whatever nixpkgs happens to ship.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Cache-friendly Rust builds: `buildDepsOnly` compiles the dependency graph
    # once into a reusable derivation, so source edits only rebuild the
    # workspace crates.
    crane.url = "github:ipetkov/crane";

    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      crane,
      flake-utils,
    }:
    let
      # nixpkgs `nixos-unstable` (26.11) has dropped x86_64-darwin entirely, so
      # the flake does not offer it rather than fail evaluation.
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
    in
    flake-utils.lib.eachSystem supportedSystems (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        inherit (pkgs) lib;
        inherit (pkgs.stdenv.hostPlatform) isLinux isDarwin;

        # Single source of truth for the version: the workspace manifest.
        version = (lib.importTOML ./Cargo.toml).workspace.package.version;

        # The toolchain the repo pins, read straight from rust-toolchain.toml.
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # ── Native inputs ────────────────────────────────────────────────────
        # The core links these libraries unconditionally: `cpal`/`alsa`,
        # `enigo`/`libxdo`, `rdev`/`libXtst`+`libevdev`, X11, `xkbcommon`, and
        # `keyring` → `libsecret`/`dbus` are unconditional dependencies even
        # when the runtime feature that uses them is off. This mirrors the apt
        # list in the Dockerfile.
        coreBuildInputs =
          with pkgs;
          [
            openssl
            libsecret
            dbus
          ]
          ++ lib.optionals isLinux [
            alsa-lib
            libevdev
            libxkbcommon
            # `enigo` probes for libxdo via pkg-config; nixpkgs ships it here.
            xdotool
            libX11
            libXtst
            libXi
            libXext
            libXrandr
            libXcursor
            libXinerama
            libxcb
          ]
          ++ lib.optionals isDarwin [
            # nixpkgs ≥ 26.11 replaced the per-framework `darwin.apple_sdk_*`
            # attributes with a single `apple-sdk_N` package that carries the
            # frameworks the stdenv setup hook exposes.
            pkgs.apple-sdk_15
          ];

        coreNativeBuildInputs =
          with pkgs;
          [
            pkg-config
            cmake
            clang
            perl
            git
          ]
          # mold is a Linux ELF linker; macOS uses its own ld64.
          ++ lib.optionals isLinux [ mold ];

        commonArgs = {
          # Guard the well-known footgun: Nix's flake-source copier drops git
          # submodule contents unless the ref asks for them, and the `vendor/`
          # crates are required to resolve the graph. Without this, a bare
          # `nix build` fails deep inside cargo with "failed to read
          # .../vendor/tinyagents/Cargo.toml"; with it, the failure names the
          # fix up front. (A `builtins.fetchGit { submodules = true; }` here
          # would fix it properly, but fetchGit is not allowed in pure
          # evaluation, and one flake input per submodule is fragile —
          # `vendor/tinyagents/vendor/tinytools` already resists it.)
          src =
            if !(builtins.pathExists ./vendor/tinyagents/Cargo.toml) then
              throw ''
                OpenHuman's vendor/ submodules are missing from the flake source.

                Nix's flake-source copier drops git submodule contents unless the
                ref asks for them. Use one of:

                  nix build "git+file://$PWD?submodules=1#openhuman-core"
                  nix build path:.#openhuman-core        # submodules already on disk

                To populate them in this checkout first:

                  bash scripts/ci/checkout-submodules.sh

                See NIX.md.
              ''
            else
              ./.;
          inherit version;
          strictDeps = true;

          nativeBuildInputs = coreNativeBuildInputs;
          buildInputs = coreBuildInputs;

          env = {
            # Use nixpkgs' OpenSSL instead of compiling it from source; the
            # sandboxed build has no network.
            OPENSSL_NO_VENDOR = "1";
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          };
        };

        # Dependencies-only build: the expensive step, cached across source
        # edits and shared by both packages below.
        cargoArtifacts = craneLib.buildDepsOnly (
          commonArgs // { cargoExtraArgs = "--locked -p openhuman-cli -p openhuman-tui"; }
        );

        mkPackage =
          {
            pname,
            cargoExtraArgs,
            description,
            mainProgram,
          }:
          craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              inherit pname;
              inherit cargoExtraArgs;
              # The repo's tests are integration-heavy (they need a mock
              # backend, Node on PATH, or a live server); leave them to the
              # project's own CI lanes rather than the package build.
              doCheck = false;
              meta = {
                inherit description mainProgram;
                homepage = "https://tinyhumans.ai/openhuman";
                license = lib.licenses.gpl3Only;
                platforms = lib.platforms.linux ++ lib.platforms.darwin;
              };
            }
          );

        openhuman-core = mkPackage {
          pname = "openhuman-core";
          cargoExtraArgs = "--locked -p openhuman-cli --bin openhuman-core";
          description = "OpenHuman core: JSON-RPC server and agent harness";
          mainProgram = "openhuman-core";
        };

        openhuman-tui = mkPackage {
          pname = "openhuman-tui";
          cargoExtraArgs = "--locked -p openhuman-tui --bin openhuman-tui";
          description = "OpenHuman terminal client (embeds the core in-process)";
          mainProgram = "openhuman-tui";
        };
      in
      {
        packages = {
          inherit openhuman-core openhuman-tui;
          default = openhuman-core;
        };

        # `nix flake check` builds these, so CI can gate the flake on the same
        # derivations users build.
        checks = {
          inherit openhuman-core openhuman-tui;
        };

        # `nix develop` — a shell that can run the repo's own commands
        # (`cargo check`, `cargo nextest`, `pnpm --filter openhuman-app build`).
        devShells.default = pkgs.mkShell {
          name = "openhuman-dev";

          nativeBuildInputs = coreNativeBuildInputs ++ [
            rustToolchain
            pkgs.nodejs_24
            pkgs.pnpm
            pkgs.cargo-nextest
          ];
          buildInputs = coreBuildInputs;

          env = {
            PKG_CONFIG_PATH = lib.makeSearchPathOutput "dev" "lib/pkg-config" coreBuildInputs;
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
            OPENSSL_NO_VENDOR = "1";
          };

          shellHook = ''
            echo "OpenHuman dev shell — Rust $(rustc --version | cut -d' ' -f2), Node $(node --version), pnpm $(pnpm --version)"
            if [ ! -e vendor/tinyagents/Cargo.toml ]; then
              echo
              echo "vendor/ submodules are not checked out; the build needs them:"
              echo "  bash scripts/ci/checkout-submodules.sh"
            fi
          '';
        };

        formatter = pkgs.nixfmt;
      }
    );
}
