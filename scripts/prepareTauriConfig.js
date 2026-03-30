export default function prepareTauriConfig() {
  // Production frontend always ships from dist; BASE_URL is for updater URLs.
  const frontendDist = '../dist';

  const config = {
    build: { frontendDist, devUrl: null },
    bundle: { windows: {} },
    identifier: 'com.openhuman.app',
  };

  if (process.env.WITH_UPDATER === 'true') {
    const repoSlug = process.env.UPDATER_REPO || 'tinyhumansai/openhuman';
    const baseUrl =
      process.env.BASE_URL ||
      `https://github.com/${repoSlug}/releases/latest/download`;
    const normalizedBaseUrl = String(baseUrl).replace(/\/+$/, '');
    const updaterEndpoint =
      process.env.UPDATER_ENDPOINT ||
      process.env.UPDATER_GIST_URL ||
      `${normalizedBaseUrl}/latest.json`;
    const updaterPublicKey = process.env.UPDATER_PUBLIC_KEY;

    if (!updaterPublicKey) {
      throw new Error(
        'WITH_UPDATER=true requires UPDATER_PUBLIC_KEY to be set',
      );
    }

    config.plugins = {
      updater: {
        dialog: false,
        endpoints: [updaterEndpoint],
        pubkey: updaterPublicKey,
      },
    };

    config.bundle.createUpdaterArtifacts = true;
  }

  if (process.env.KEYPAIR_ALIAS) {
    config.bundle.windows.signCommand = `smctl.exe sign --keypair-alias=${process.env.KEYPAIR_ALIAS} --input %1`;
  }

  return config;
}
