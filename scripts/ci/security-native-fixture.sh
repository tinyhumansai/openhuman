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
release_source="$(git -C vendor/tinysecurity rev-parse 24921baa4cc0f92375d1d384a945429998e529d0)"
printf 'TinySecurity pin: %s\nPublished v0.20.0 source: %s\n' "$security_pin" "$release_source"
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
# Download a registry-pinned release, check its digest and stage it; leaves the
# staged directory in $module_dir.
fetch_bundled() {
  local id="$1" url="$2" archive="$3" sha="$4" path version
  path="$OPENHUMAN_TEST_SECURITY_FIXTURE_DIR/$archive"
  curl --fail --location --silent --show-error --retry 4 --retry-delay 2 \
    "$url" --output "$path"
  node - "$path" "$sha" <<'NODE'
const { createHash } = require("node:crypto");
const { readFileSync } = require("node:fs");

const [file, expected] = process.argv.slice(2);
const actual = createHash("sha256").update(readFileSync(file)).digest("hex");
if (actual !== expected) {
  throw new Error(`module fixture digest mismatch: expected ${expected}, got ${actual}`);
}
NODE
  version="${archive#"$id"-module-}"
  version="${version#"$id"-}"
  version="${version%%-"$host_key"*}"
  stage_bundled "$id" "$version" "$path" "$archive"
  module_dir="$bundled_dir/$id/$version/$host_key"
}
stage_bundled tinysecurity "$release_version" "$archive_path" "$archive"
env -u OPENHUMAN_MODULE_PATH OPENHUMAN_BUNDLED_MODULES="$bundled_dir" \
  bash scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman-cli \
  --test security_release_reuse_fixture --no-default-features \
  --features "$core_features,security-module" \
  native_fixture_serves_later_calls_after_a_release_admission \
  -- --ignored --exact --nocapture

# The callback-bound modules load the same way in the shipped app: a pinned
# release with no local-file source. Stage each verified archive as a bundled
# module and check that it admits and reloads.
callback_modules="tinymcp,tinyeventqueues,tinychannels,tinycomputer"
loadable_modules=""
while IFS=$'\t' read -r module_id module_url module_archive module_sha; do
  fetch_bundled "$module_id" "$module_url" "$module_archive" "$module_sha"
  # A Linux module that links a system library this runner lacks (TinyComputer
  # links libxdo and libasound) cannot be dlopened here at all; say so and
  # check the rest rather than fail on the runner's package set.
  if [[ "$(uname -s)" == Linux ]]; then
    missing="$(find "$module_dir" -name '*.so' -exec ldd {} + | awk '/not found/ {print $1}' | sort -u | tr '\n' ' ')"
    if [[ -n "$missing" ]]; then
      echo "callback module fixture: skipping $module_id; this runner lacks: $missing" >&2
      continue
    fi
  fi
  loadable_modules="${loadable_modules:+$loadable_modules,}$module_id"
done < <(node scripts/ci/self-hosted/test-module-assets.mjs "$host_key" "$callback_modules")
echo "callback module fixture: checking $loadable_modules" >&2
test -n "$loadable_modules"
env -u OPENHUMAN_MODULE_PATH OPENHUMAN_BUNDLED_MODULES="$bundled_dir" \
  OPENHUMAN_TEST_CALLBACK_MODULES="$loadable_modules" \
  OPENHUMAN_KEYRING_MASTER_KEY="$(node -e 'process.stdout.write(require("node:crypto").randomBytes(32).toString("hex"))')" \
  bash scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman-cli \
  --test callback_module_release_fixture --no-default-features \
  --features "$core_features,security-module" \
  native_fixture_loads_callback_modules_from_a_release_source \
  -- --ignored --exact --nocapture

# Desktop sign-in, end to end: the desktop core with TinySecurity from its
# bundled release and no socket stores the session token in the OS keyring
# once the keyring prompt is approved, and stays signed out when it is denied.
# TinySecurity's Linux build keeps keyring entries in the Secret Service, so
# each run gets a private D-Bus session and an unlocked throwaway keyring. A
# runner without one skips it, as above for missing system libraries, unless
# OPENHUMAN_TEST_REQUIRE_KEYRING_E2E=1 says this runner must run it.
sign_in_fixture() {
  local keyring_home
  keyring_home="$(mktemp -d)"
  mkdir -p "$keyring_home/data" "$keyring_home/run" && chmod 700 "$keyring_home/run"
  env -u OPENHUMAN_MODULE_PATH OPENHUMAN_BUNDLED_MODULES="$bundled_dir" \
    XDG_DATA_HOME="$keyring_home/data" XDG_RUNTIME_DIR="$keyring_home/run" \
    OPENHUMAN_TEST_SIGN_IN_WORKSPACE="$keyring_home/workspace" \
    OPENHUMAN_KEYRING_MASTER_KEY="$(node -e 'process.stdout.write(require("node:crypto").randomBytes(32).toString("hex"))')" \
    FIXTURE_FEATURES="$core_features,security-module" \
    dbus-run-session -- bash -c '
      set -euo pipefail
      printf "fixture-keyring" | gnome-keyring-daemon --unlock --components=secrets >/dev/null
      # The cases share one keyring and one workspace, in order.
      for test_name in "$@"; do
        bash scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman-cli \
          --test desktop_sign_in_fixture --no-default-features --features "$FIXTURE_FEATURES" \
          "$test_name" -- --ignored --exact --nocapture
      done
      # An approved sign-in leaves its entry in the OS keyring; a denied one does not.
      if command -v secret-tool >/dev/null; then
        echo "Secret Service entries for service=openhuman:"
        secret-tool search --all service openhuman 2>&1 | grep -E "^(label|attribute)" || echo "(none)"
      else
        echo "secret-tool is not installed; the keyring listing is skipped"
      fi
    ' sign-in-fixture "$@"
  rm -rf "$keyring_home"
}
sign_in_missing=""
if [[ "$(uname -s)" != Linux ]]; then
  sign_in_missing="a Linux Secret Service"
else
  for tool in gnome-keyring-daemon dbus-run-session; do
    command -v "$tool" >/dev/null || sign_in_missing="${sign_in_missing:+$sign_in_missing }$tool"
  done
fi
if [[ -n "$sign_in_missing" ]]; then
  echo "desktop sign-in E2E: skipping; this runner lacks: $sign_in_missing" >&2
  if [[ "${OPENHUMAN_TEST_REQUIRE_KEYRING_E2E:-}" == 1 ]]; then
    echo "desktop sign-in E2E: OPENHUMAN_TEST_REQUIRE_KEYRING_E2E=1, so a skip is a failure" >&2
    exit 1
  fi
else
  # Signing in prunes retired cron jobs through TinyFlows before it commits
  # (config/user_scope.rs), and the shipped app bundles TinyFlows too.
  while IFS=$'\t' read -r module_id module_url module_archive module_sha; do
    fetch_bundled "$module_id" "$module_url" "$module_archive" "$module_sha"
  done < <(node scripts/ci/self-hosted/test-module-assets.mjs "$host_key" tinyflows)
  sign_in_fixture desktop_sign_in_stores_the_token_in_the_os_keyring_after_approval \
    desktop_restart_reads_the_keyring_token_once_approved_at_startup
  sign_in_fixture desktop_sign_in_denied_at_the_keyring_prompt_leaves_the_device_signed_out
fi
