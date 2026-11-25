//! Registry record for the `tinyjuice` module.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYJUICE: ModuleRecord = ModuleRecord {
    id: "tinyjuice",
    description: "Content-aware tool-output compression and recoverable caching",
    bus_name: tinyjuice_bus::names::BUS_NAME,
    object_path: tinyjuice_bus::names::OBJECT_PATH,
    version: "0.11.6",
    release_url: "https://github.com/tinyhumansai/tinyjuice/releases/tag/v0.11.6",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinyjuice-module-0.11.6-archlinux-arm64.tar.gz",
            sha256: "54151837611c5a7239873fc8e7bc5d8c74f1f0818d78ee58ba508dd5f15e58d9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinyjuice-module-0.11.6-archlinux-x86_64.tar.gz",
            sha256: "a451f018fc81067bcb49db5aaf7265ebab1b7d8c9750fe9b7cca2405667859b7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyjuice-module-0.11.6-fedora-43-arm64.tar.gz",
            sha256: "e6dc6550c5df5e1db17cf84b000a8cc893bae2b3082ed9b49813c01ea5e06104",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyjuice-module-0.11.6-fedora-43-x86_64.tar.gz",
            sha256: "9b7565e6b1f9fac6a4007d2494a5224f3e2f8f46b7c54d0eeede5b20804fc86c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyjuice-module-0.11.6-fedora-44-arm64.tar.gz",
            sha256: "e1a780719502a6bff4d42bca28353e12a750c427255ad39c197ec00379d5ea1d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyjuice-module-0.11.6-fedora-44-x86_64.tar.gz",
            sha256: "eee03fdb25ebc69c84431834e2c2e9c528a81b32bd92a20034d04f9680ef47e8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyjuice-module-0.11.6-macos-15-arm64.tar.gz",
            sha256: "35157f06332c06675d92a3c4392030d1a6b72de6fc7c8fd8ddd97390d33d1079",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyjuice-module-0.11.6-macos-15-x86_64.tar.gz",
            sha256: "519560c5342d8853e6c4f1f1f33f5cdd0bb10ca537be92c4349509fd504f78d7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyjuice-module-0.11.6-macos-26-arm64.tar.gz",
            sha256: "0a73b15bf14fcfeb676ab2b70e842e15f61fa97595faeefbcd4608aaf98e5e6e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyjuice-module-0.11.6-macos-26-x86_64.tar.gz",
            sha256: "3a3dfcc4d2139de8a936d058f1c87c8c6dc201bb903566ea72061f21281f73ae",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyjuice-module-0.11.6-ubuntu-22.04-arm64.tar.gz",
            sha256: "c940e64939e03260ae4d8434dae10d8607857aa26a0ebf05456f2b7d3b485f52",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyjuice-module-0.11.6-ubuntu-22.04-x86_64.tar.gz",
            sha256: "7ed6b56a6972f6aa8661899e0489f09883c791fad87b01fd29a9e52e9cd4c409",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyjuice-module-0.11.6-ubuntu-24.04-arm64.tar.gz",
            sha256: "a819f7c1f77ffc8887d8cd1ecf9d7bc1dafdbcbaa4fee5b3389c6a9292eb2699",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyjuice-module-0.11.6-ubuntu-24.04-x86_64.tar.gz",
            sha256: "2dfa5d532a966460aa12d74f4cdaf7f4e379233682a9981831e2605b397172ea",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyjuice-module-0.11.6-windows-11-arm64.zip",
            sha256: "04af7fdd36e836f8cea86ca1d36e37fdf09147fad433e15b9a18cce472fdd4d0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyjuice-module-0.11.6-windows-2022-x86_64.zip",
            sha256: "8a510083d6b0350c872915c2a89bca4a013d5231552a56311922f3b291699633",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyjuice-module-0.11.6-windows-2025-x86_64.zip",
            sha256: "0236cb968893c28320cc04626b20fdf0140ea3d45cc5872f1164bc6704883038",
            library_sha256: None,
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinyjuice",
        manifest: "crates/tinyjuice-module/Cargo.toml",
        package: "tinyjuice-module",
        lib_name: "tinyjuice_module",
        features: &[],
    },
};
