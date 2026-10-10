//! Registry records for additional first-party TinyBus modules.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset};

/// The `tinybox` module, loaded on demand.
pub(crate) const TINYBOX: ModuleRecord = ModuleRecord {
    id: "tinybox",
    description: "Sandbox capability discovery through TinyBox",
    bus_name: "ai.tinyhumans.tinybox.Box",
    object_path: "/ai/tinyhumans/tinybox/Box",
    version: "0.1.17",
    release_url: "https://github.com/tinyhumansai/tinybox/releases/tag/v0.1.17",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinybox-0.1.17-ubuntu-24.04-x86_64.tar.gz",
            sha256: "be13b54f9d6a039c9de484f63363f647d8c3d2d8f0cc14b768da83372941ce5a",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinybox-0.1.17-ubuntu-24.04-arm64.tar.gz",
            sha256: "0489d5e8591c1a04f2733f0178495b2c232eecec1e3aee3ff67fe2248bc59b68",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinybox-0.1.17-ubuntu-22.04-x86_64.tar.gz",
            sha256: "a14eba80278bc311e67cd19db174a66f39a519663f5f07f9d0ebcf14b34d89e4",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinybox-0.1.17-ubuntu-22.04-arm64.tar.gz",
            sha256: "3db57724a935e3e859afc107850a31b51df6f5794467754163ab8a4c1bf869ec",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinybox-0.1.17-macos-26-arm64.tar.gz",
            sha256: "6e285f3505664ef6aa59d683542320384c6036b3b858d1d67177242587ed305b",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinybox-0.1.17-macos-26-x86_64.tar.gz",
            sha256: "bae82655ed7301ba1b9a2caa427ccd53924195821326305edfa19b9ebcb3f0c4",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinybox-0.1.17-macos-15-arm64.tar.gz",
            sha256: "b531f41a8266c59a2240f139917838e4185dbe80642626e56da2ebb4814dd5d9",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinybox-0.1.17-macos-15-x86_64.tar.gz",
            sha256: "7dbbd9ea55bff99ac554032ba26963672db8cc77644059de89b3d2739c05fbd2",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinybox-0.1.17-windows-2025-x86_64.zip",
            sha256: "3c4546481875de85a8fed6b9e0cab78c4af6b83a2958de513284b5d5b7f930d0",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinybox-0.1.17-windows-2022-x86_64.zip",
            sha256: "86cc55e9e9abe895eab0c4cd547036330f5cef7a0680ab88c1087d531a929900",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinybox-0.1.17-windows-11-arm64.zip",
            sha256: "0fb3f7c2df91ea4027771f877fcec8456ca12675295c2bce3694e89e50c25266",
        },
    ],
    load: LoadPolicy::Lazy,
};

/// The `tinychannels` module, loaded on demand.
pub(crate) const TINYCHANNELS: ModuleRecord = ModuleRecord {
    id: "tinychannels",
    description: "Channel provider lifecycle and message transport",
    bus_name: "ai.tinyhumans.tinychannels.Channels",
    object_path: "/ai/tinyhumans/tinychannels/Channels",
    version: "0.1.13",
    release_url: "https://github.com/tinyhumansai/tinychannels/releases/tag/v0.1.13",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinychannels-module-0.1.13-ubuntu-24.04-x86_64.tar.gz",
            sha256: "3fe360f6863a01e2fe48737d9895fd997467121ee1fa9bb30e7f875368c9b5c9",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinychannels-module-0.1.13-ubuntu-24.04-arm64.tar.gz",
            sha256: "e636f26b9484b351409f1c1ac0f22ee61435a24961cb0f57e2b7537774b0934a",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinychannels-module-0.1.13-ubuntu-22.04-x86_64.tar.gz",
            sha256: "f0ca5091e42c1493311db34015d4485a557700100feac4245468c3d0f64eef08",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinychannels-module-0.1.13-ubuntu-22.04-arm64.tar.gz",
            sha256: "2824102014b345c21c9d7e16fae8917cdf6fea03529a67de1ee58065f1a694e6",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinychannels-module-0.1.13-macos-26-arm64.tar.gz",
            sha256: "81e16aa479839a673b03a18bd715ebc1b5efef7e6f223953233688ad08d1bbf2",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinychannels-module-0.1.13-macos-26-x86_64.tar.gz",
            sha256: "dc82f10309946132e7837fb97c86203d79741416536b0d944972cb8acaa3f220",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinychannels-module-0.1.13-macos-15-arm64.tar.gz",
            sha256: "7770a906ac3bb46e89725363d723e32709b45300d234b360c4822666c29f7c51",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinychannels-module-0.1.13-macos-15-x86_64.tar.gz",
            sha256: "cbd540871efc81822181ddd51502c93f31e2a99a8be58b725d01bc70d7d9ba6d",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinychannels-module-0.1.13-windows-2025-x86_64.zip",
            sha256: "a7fee0a4bcbb52b4f7866eab6a1c4eca31fa7de8541c8321c61314c1286f981a",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinychannels-module-0.1.13-windows-2022-x86_64.zip",
            sha256: "1a74d967ac4f859d6c210865788fcdb124accc027e75823eb3d9a55a700a0dcd",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinychannels-module-0.1.13-windows-11-arm64.zip",
            sha256: "cbcaaeed4c6e2c3baae1474686e210281c16d8f75024d1a4805f68cbbc8f1f67",
        },
    ],
    load: LoadPolicy::Lazy,
};

/// The `tinyhosts` module, loaded on demand.
pub(crate) const TINYHOSTS: ModuleRecord = ModuleRecord {
    id: "tinyhosts",
    description: "Hosting provider operations",
    bus_name: "ai.tinyhumans.tinyhosts.Hosting",
    object_path: "/ai/tinyhumans/tinyhosts/Hosting",
    version: "0.2.3",
    release_url: "https://github.com/tinyhumansai/tinyhosts/releases/tag/v0.2.3",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyhosts-0.2.3-ubuntu-24.04-x86_64.tar.gz",
            sha256: "a0e623c5f541c1d21f2e7aba16ce3074fcee06f014c39c43fd2a7f7a3a77f6dd",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyhosts-0.2.3-ubuntu-24.04-arm64.tar.gz",
            sha256: "0e0b0467c08e3f79275fbbd46e84df3dea570f896b746031a581d030eb7eb9c5",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyhosts-0.2.3-ubuntu-22.04-x86_64.tar.gz",
            sha256: "6d622d9497ebdea67323125e25dab0d6497e0d3f39c351f3ef13de016076fc8b",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyhosts-0.2.3-ubuntu-22.04-arm64.tar.gz",
            sha256: "6648fa3debb518ab5090aef2e49a3eed147ca00eef5c0c7e22576768cb7d1f51",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyhosts-0.2.3-macos-26-arm64.tar.gz",
            sha256: "8c8d62564df5fad2b3d4da30e56e86969584aacb30a22875f96a6209d1e0cfae",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyhosts-0.2.3-macos-26-x86_64.tar.gz",
            sha256: "801cec78dfe01006ff6811b445730de3f246087f1cbe9f0d63da8bcf55bf3f5d",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyhosts-0.2.3-macos-15-arm64.tar.gz",
            sha256: "10b9ac5b6e414064b4d9abb9cc4cb47b7a6e80e0cbfd0c29a8b1afe41637fb0f",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyhosts-0.2.3-macos-15-x86_64.tar.gz",
            sha256: "3b9b608d4fa3b638c93485fefca3825ebdc4220d0fc4dc2312f94a570dadcb83",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyhosts-0.2.3-windows-2025-x86_64.zip",
            sha256: "b9caaed8b6b2235098c88477d8101f2dcd26e4d5a68109957836272616f25b71",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyhosts-0.2.3-windows-2022-x86_64.zip",
            sha256: "a1ca38023401efd5fda4db6f8b30701dd9a209563f74d5a961e6572e7c4b6223",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyhosts-0.2.3-windows-11-arm64.zip",
            sha256: "3912ad5a10ce87e2de5e91e29f8beb1ffb669e31ca2dcdd7e0e522b63607adf7",
        },
    ],
    load: LoadPolicy::Lazy,
};
