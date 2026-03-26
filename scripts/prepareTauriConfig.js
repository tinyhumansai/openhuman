export default function prepareTauriConfig() {
  // For production builds, use the dist directory path
  // BASE_URL is only used for updater endpoints, not for frontendDist
  const frontendDist = process.env.BASE_URL?.startsWith('http')
    ? '../dist'
    : process.env.BASE_URL || '../dist';

  const config = {
    build: { frontendDist, devUrl: null },
    bundle: { windows: {} },
    identifier: 'com.openhuman.app',
  };

  if (process.env.WITH_UPDATER === 'true') {
    config.plugins = {
      updater: {
        dialog: false,
        endpoints: [process.env.UPDATER_GIST_URL],
        pubkey: process.env.UPDATER_PUBLIC_KEY,
      },
    };

    config.bundle.createUpdaterArtifacts = true;
  }

  if (process.env.KEYPAIR_ALIAS) {
    config.bundle.windows.signCommand = `smctl.exe sign --keypair-alias=${process.env.KEYPAIR_ALIAS} --input %1`;
  }

  return config;
}
