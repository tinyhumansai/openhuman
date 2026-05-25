# Mobile app icons

Brand-quality icons committed to the repo so initial `tauri ios init` /
`tauri android init` runs produce a real-looking app instead of the
placeholder Tauri ships.

| Path | Used by |
| --- | --- |
| `icon.png` (1024×1024) | `tauri.conf.json#bundle.icon` — Tauri build pipeline |
| `ios/AppIcon.appiconset/*` | Copied by `scripts/ios-init.sh` into `gen/apple/<bundle>_iOS/Assets.xcassets/AppIcon.appiconset/` after init |
| `android/mipmap-{m,h,xh,xxh,xxxh}dpi/ic_launcher.png` | Copied by `scripts/android-init.sh` into `gen/android/app/src/main/res/mipmap-*/` after init |
| `store/appstore.png` (1024×1024) | App Store Connect upload |
| `store/playstore.png` (512×512) | Google Play Console upload |

The `gen/` directory is `.gitignore`d (Tauri regenerates it from
`tauri.conf.json` on every `init`), so the canonical source for icons
must live here, not under `gen/`.
