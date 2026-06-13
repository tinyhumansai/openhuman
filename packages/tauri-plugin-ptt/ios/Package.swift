// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "tauri-plugin-ptt",
    platforms: [
        .iOS(.v16),
    ],
    products: [
        .library(
            name: "tauri-plugin-ptt",
            type: .static,
            targets: ["tauri-plugin-ptt"]
        ),
    ],
    dependencies: [
        .package(name: "Tauri", path: "../.tauri/tauri-api"),
    ],
    targets: [
        .target(
            name: "tauri-plugin-ptt",
            dependencies: [
                .product(name: "Tauri", package: "Tauri"),
            ],
            path: "Sources/tauri-plugin-ptt"
        ),
    ]
)
