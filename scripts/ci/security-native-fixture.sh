#!/usr/bin/env bash
set -euo pipefail

# Native integration test loads the published archive through the compiled
# registry digest. The fixture is intentionally isolated from ordinary tests.
export OPENHUMAN_TEST_SECURITY_FIXTURE_DIR="${OPENHUMAN_TEST_SECURITY_FIXTURE_DIR:-$HOME/.cache/openhuman-security-fixtures}"
mkdir -p "$OPENHUMAN_TEST_SECURITY_FIXTURE_DIR"
chmod 700 "$OPENHUMAN_TEST_SECURITY_FIXTURE_DIR"

host_key="${OH_MODULE_HOST_KEY:-}"
if [ -z "$host_key" ]; then
  case "$(uname -s)" in
    Linux) host_key="ubuntu-24.04-x86_64" ;;
    Darwin)
      case "$(uname -m)" in
        arm64) host_key="macos-15-arm64" ;;
        *) host_key="macos-15-x86_64" ;;
      esac
      ;;
    MINGW*|MSYS*|CYGWIN*) host_key="windows-2025-x86_64" ;;
    *) echo "unsupported native fixture host: $(uname -s)" >&2; exit 1 ;;
  esac
fi

asset="$(node scripts/ci/self-hosted/test-module-assets.mjs "$host_key" tinysecurity)"
IFS=$'\t' read -r _module_id asset_url archive expected_sha <<< "$asset"
archive_path="$OPENHUMAN_TEST_SECURITY_FIXTURE_DIR/$archive"
module_dir="$OPENHUMAN_TEST_SECURITY_FIXTURE_DIR/tinysecurity"
curl --fail --location --silent --show-error --retry 4 --retry-delay 2 \
  "$asset_url" --output "$archive_path"
node - "$archive_path" "$expected_sha" <<'NODE'
const { createHash } = require("node:crypto");
const { readFileSync } = require("node:fs");

const [file, expected] = process.argv.slice(2);
const actual = createHash("sha256").update(readFileSync(file)).digest("hex");
if (actual !== expected) {
  throw new Error(`TinySecurity fixture digest mismatch: expected ${expected}, got ${actual}`);
}
NODE
rm -rf "$module_dir"
mkdir -p "$module_dir"
if [[ "$archive" == *.zip ]]; then
  powershell.exe -NoProfile -Command \
    "Expand-Archive -LiteralPath '$(cygpath -w "$archive_path")' -DestinationPath '$(cygpath -w "$module_dir")' -Force"
else
  tar --no-same-owner -xzf "$archive_path" -C "$module_dir"
fi
export OPENHUMAN_MODULE_PATH="$module_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"

core_features="$(scripts/ci/product-features.sh | sed 's/,os-keyring//' | sed 's/,security-module//')"

security_pin="$(git -C vendor/tinysecurity rev-parse HEAD)"
release_source="$(git -C vendor/tinysecurity rev-parse b53eceb948bd8dedce2176af6f0ec7bb1bd74151)"
printf 'TinySecurity pin: %s\nPublished v0.18.4 source: %s\n' "$security_pin" "$release_source"
git -C vendor/tinysecurity merge-base --is-ancestor "$release_source" "$security_pin"

# Prove the library builds without the new gate before exercising it enabled.
bash scripts/ci-cancel-aware.sh cargo check --manifest-path Cargo.toml -p openhuman \
  --no-default-features --features "$core_features"

bash scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman-cli \
  --test security_native_fixture --no-default-features \
  --features "$core_features,security-module" \
  native_fixture_exercises_the_pinned_release_contracts \
  -- --ignored --exact --nocapture

# Corrupt a copy only after validating the official archive checksum. The
# explicit fixture path is a test-only module source override.
tampered_dir="$OPENHUMAN_TEST_SECURITY_FIXTURE_DIR/tinysecurity-tampered"
cp -R "$module_dir" "$tampered_dir"
module_library="$(find "$tampered_dir" -type f \( -name '*.so' -o -name '*.dylib' -o -name '*.dll' \) -print -quit)"
test -n "$module_library"
printf '\n' >> "$module_library"
OPENHUMAN_MODULE_PATH="$tampered_dir" bash scripts/ci-cancel-aware.sh cargo test \
  --manifest-path Cargo.toml -p openhuman-cli --test security_native_fixture \
  --no-default-features --features "$core_features,security-module" \
  native_fixture_rejects_a_tampered_module -- --ignored --exact --nocapture

bash scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman-cli \
  --test security_native_fixture --no-default-features \
  --features "$core_features,security-module" \
  native_fixture_denies_keyring_before_backend_access \
  -- --ignored --exact --nocapture

bash scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman-cli \
  --test security_native_fixture --no-default-features \
  --features "$core_features,security-module" \
  native_fixture_sandbox_plan_is_audited_before_running \
  -- --ignored --exact --nocapture

bash scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman-cli \
  --test security_native_fixture --no-default-features \
  --features "$core_features,security-module" \
  native_fixture_parks_a_redacted_approval_until_denied \
  -- --ignored --exact --nocapture

bash scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman-cli \
  --test security_native_fixture --no-default-features \
  --features "$core_features,security-module" \
  native_fixture_refuses_a_sandbox_plan_when_the_audit_write_fails \
  -- --ignored --exact --nocapture

bash scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman-cli \
  --test security_native_fixture --no-default-features \
  --features "$core_features,security-module" \
  native_fixture_denies_an_approval_when_the_audit_write_fails \
  -- --ignored --exact --nocapture

# TinySecurity keeps the secret scope it was admitted with for the process. An
# embedded runtime built after an earlier one was released takes the module
# over; another scope is refused while the admitting runtime is live.
# The facade's default features on purpose: they reach TinySecurity through
# the core's defaults without naming the facade's own `security-module`.
bash scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman-embed \
  --test security_second_runtime \
  a_runtime_built_after_another_is_released_still_screens_its_prompts \
  -- --ignored --exact --nocapture

bash scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman-embed \
  --test security_live_scope --no-default-features \
  --features "$core_features,security-module" \
  a_different_secret_scope_is_refused_while_the_admitting_runtime_is_live \
  -- --ignored --exact --nocapture

# The shipped app loads TinySecurity as a pinned release, never from a local
# file. Present the same verified archive as a bundled module instead, with no
# local-file source, so later loads exercise the retained callback binding.
release_version="${archive#tinysecurity-module-}"
release_version="${release_version%%-"$host_key"*}"
bundled_dir="$OPENHUMAN_TEST_SECURITY_FIXTURE_DIR/bundled"
rm -rf "$bundled_dir"
# Lay a verified archive out as an installer bundle does
# (scripts/release/stage-modules.mjs): `<id>/<version>/<host_key>/` holding the
# archive and its extracted module side by side.
stage_bundled() {
  local id="$1" version="$2" source="$3" name="$4"
  local dir="$bundled_dir/$id/$version/$host_key"
  mkdir -p "$dir"
  cp "$source" "$dir/$name"
  if [[ "$name" == *.zip ]]; then
    powershell.exe -NoProfile -Command \
      "Expand-Archive -LiteralPath '$(cygpath -w "$dir/$name")' -DestinationPath '$(cygpath -w "$dir")' -Force"
  else
    tar --no-same-owner -xzf "$dir/$name" -C "$dir"
  fi
}
stage_bundled tinysecurity "$release_version" "$archive_path" "$archive"
env -u OPENHUMAN_MODULE_PATH OPENHUMAN_BUNDLED_MODULES="$bundled_dir" \
  bash scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman-cli \
  --test security_release_reuse_fixture --no-default-features \
  --features "$core_features,security-module" \
  native_fixture_serves_later_calls_after_a_release_admission \
  -- --ignored --exact --nocapture
