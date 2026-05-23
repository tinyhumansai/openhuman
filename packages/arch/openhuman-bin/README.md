# openhuman-bin AUR package

This directory contains the Arch Linux `openhuman-bin` package recipe. It uses
the official x86_64 AppImage from GitHub Releases as the binary source, extracts
its bundled application tree during `makepkg`, installs a desktop entry, and
adds `/usr/bin/openhuman` as a launcher.

The package does not launch the AppImage runtime directly. Arch-family distros
have reported `Interpreter not found!` from the bundled AppImage runtime on
v0.54.0, so the launcher executes the extracted `shared/bin/OpenHuman` binary
with the bundled library path instead.

## Local package test

From this directory on an Arch Linux host:

```bash
makepkg --syncdeps --clean --cleanbuild --force
pacman -Qip openhuman-bin-*.pkg.tar.zst
```

## Release bump

1. Set `pkgver` to the new stable release version without the leading `v`.
2. Update the AppImage SHA-256 checksum from the GitHub release asset.
3. Update the launcher checksum if `openhuman` changes.
4. Regenerate `.SRCINFO` before publishing to AUR:

```bash
updpkgsums
makepkg --printsrcinfo > .SRCINFO
```

The AUR repository should contain `PKGBUILD`, `.SRCINFO`, `openhuman`,
`openhuman.desktop`, and `openhuman.svg`.
