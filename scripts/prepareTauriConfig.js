// Tauri config overrides applied at CI build time on top of the static
// `app/src-tauri/tauri.conf.json`. Anything returned here is merged via
// `tauri build --config <json>` and wins over the static file.
//
// History note: this file used to inject `plugins.updater.pubkey` and
// `plugins.updater.endpoints` from `UPDATER_PUBLIC_KEY` / `UPDATER_ENDPOINT`
// env vars sourced from GitHub secrets. That indirection caused a real
// outage class — if the build-time pubkey ever drifted out of sync with the
// `TAURI_SIGNING_PRIVATE_KEY` secret used to sign artifacts, every signed
// installer was rejected by its own embedded pubkey at install time
// ("bad keys"). The static `app/src-tauri/tauri.conf.json` is now the
// single source of truth for the updater pubkey + endpoint; rotate via
// commit + review instead of silent secret swaps.
//
// What's left at build time:
//   - `WITH_UPDATER=true` → flip `bundle.createUpdaterArtifacts` on so
//     the bundler emits signed `.app.tar.gz` / `.sig` artifacts. Only the
//     release pipeline sets this; PR builds (`build.yml`,
//     `build-windows.yml`, `test.yml`) leave it unset and skip artifact
//     signing entirely (those jobs don't have `TAURI_SIGNING_PRIVATE_KEY`
//     access by design).
//   - `KEYPAIR_ALIAS` → Windows DigiCert SmartCard sign command. Has to
//     stay build-time because the alias is a runner secret.
export default function prepareTauriConfig() {
  const config = {};
  const bundle = {};

  if (process.env.WITH_UPDATER === 'true') {
    bundle.createUpdaterArtifacts = true;
  }

  if (process.env.KEYPAIR_ALIAS) {
    bundle.windows = {
      signCommand: `smctl.exe sign --keypair-alias=${process.env.KEYPAIR_ALIAS} --input %1`,
    };
  }

  if (Object.keys(bundle).length > 0) {
    config.bundle = bundle;
  }

  return config;
}
