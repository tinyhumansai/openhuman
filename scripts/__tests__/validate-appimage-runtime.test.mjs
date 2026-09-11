// Regression guard for #5606.
//
// The AppImage bundler silently changed underneath these scripts when the app
// moved off CEF (#5456): the vendored CEF-aware tauri-cli bundled via sharun,
// stock tauri-bundler bundles via linuxdeploy. The validator kept asserting a
// sharun layout and hard-failed every Linux release for eleven days, and no
// pre-merge lane exercised it because `build-desktop.yml` is only ever called
// from the two release workflows.
//
// These tests pin the layout classifier and the linuxdeploy structural
// assertions against synthetic AppDirs, so a future bundler swap fails here
// rather than at release time.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const HERE = dirname(fileURLToPath(import.meta.url));
const VALIDATOR = resolve(
  HERE,
  "..",
  "release",
  "validate-appimage-runtime.sh",
);

// Bash-only; the scripts under test are bash and the release runners are Linux.
const SKIP =
  process.platform === "win32" ? { skip: "requires a POSIX shell" } : {};

function elf(path) {
  // Enough of an ELF header for the magic-byte checks in is_elf/is_executable_elf.
  fs.writeFileSync(
    path,
    Buffer.from([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
  );
  fs.chmodSync(path, 0o755);
}

function script(path, body) {
  fs.writeFileSync(path, body, { mode: 0o755 });
}

/** Build the AppDir that tauri-bundler 2.9.4 + linuxdeploy actually produce. */
function makeLinuxdeployAppDir() {
  const root = fs.mkdtempSync(join(os.tmpdir(), "openhuman-appdir-ld-"));
  const app = join(root, "OpenHuman.AppDir");
  for (const d of [
    "usr/bin",
    "usr/lib",
    "usr/share/applications",
    "usr/share/icons/hicolor/256x256@2/apps",
    "apprun-hooks",
  ]) {
    fs.mkdirSync(join(app, d), { recursive: true });
  }

  // AppRun is a bash wrapper written by linuxdeploy; the AppImageKit ELF is
  // renamed to AppRun.wrapped. Asserting AppRun is an ELF can never pass.
  script(
    join(app, "AppRun"),
    [
      "#! /usr/bin/env bash",
      "set -e",
      'this_dir="$(readlink -f "$(dirname "$0")")"',
      'source "$this_dir"/apprun-hooks/linuxdeploy-plugin-gtk.sh',
      'exec "$this_dir"/AppRun.wrapped "$@"',
      "",
    ].join("\n"),
  );
  script(join(app, "apprun-hooks/linuxdeploy-plugin-gtk.sh"), "#!/bin/sh\n");
  elf(join(app, "AppRun.wrapped"));
  elf(join(app, "usr/bin/OpenHuman"));

  // xdg-mime / xdg-open are copied in as POSIX shell scripts, not ELF (#5607).
  script(join(app, "usr/bin/xdg-mime"), "#!/bin/sh\n");
  script(join(app, "usr/bin/xdg-open"), "#!/bin/sh\n");

  elf(join(app, "usr/lib/libxdo.so.3"));
  elf(join(app, "usr/lib/libwebkit2gtk-4.1.so.0"));

  fs.writeFileSync(
    join(app, "usr/share/applications/OpenHuman.desktop"),
    [
      "[Desktop Entry]",
      "Exec=OpenHuman --enable-features=UseOzonePlatform --ozone-platform=x11",
      "Icon=OpenHuman",
      "Name=OpenHuman",
      "Type=Application",
      "MimeType=x-scheme-handler/openhuman",
      "",
    ].join("\n"),
  );

  const icon = "usr/share/icons/hicolor/256x256@2/apps/OpenHuman.png";
  fs.writeFileSync(join(app, icon), "");
  // The AppDir-root icon and .desktop are SYMLINKS, not regular files.
  fs.symlinkSync(icon, join(app, "OpenHuman.png"));
  fs.symlinkSync("OpenHuman.png", join(app, ".DirIcon"));
  fs.symlinkSync(
    "usr/share/applications/OpenHuman.desktop",
    join(app, "OpenHuman.desktop"),
  );

  return { root, app };
}

/** Build the pre-Wry sharun AppDir, which must still validate. */
function makeSharunAppDir() {
  const root = fs.mkdtempSync(join(os.tmpdir(), "openhuman-appdir-sharun-"));
  const app = join(root, "OpenHuman.AppDir");
  for (const d of ["shared/bin", "shared/lib", "bin"]) {
    fs.mkdirSync(join(app, d), { recursive: true });
  }
  elf(join(app, "sharun"));
  fs.copyFileSync(join(app, "sharun"), join(app, "AppRun"));
  fs.chmodSync(join(app, "AppRun"), 0o755);
  fs.copyFileSync(join(app, "sharun"), join(app, "bin/OpenHuman"));
  fs.chmodSync(join(app, "bin/OpenHuman"), 0o755);
  elf(join(app, "shared/bin/OpenHuman"));
  fs.writeFileSync(join(app, "shared/lib/lib.path"), "lib\n");
  return { root, app };
}

function bash(snippet) {
  return spawnSync(
    "bash",
    ["-c", `source ${JSON.stringify(VALIDATOR)} >/dev/null 2>&1\n${snippet}`],
    {
      encoding: "utf8",
    },
  );
}

function layoutOf(appdir) {
  return bash(`appdir_layout ${JSON.stringify(appdir)} || true`).stdout.trim();
}

function validateLinuxdeploy(appdir) {
  return (
    bash(`validate_linuxdeploy_appdir ${JSON.stringify(appdir)}`).status === 0
  );
}

test(
  "classifies a linuxdeploy AppDir and resolves its binary from Exec=",
  SKIP,
  () => {
    const { root, app } = makeLinuxdeployAppDir();
    try {
      assert.equal(layoutOf(app), "linuxdeploy");
      const main = bash(
        `appdir_main_binary ${JSON.stringify(app)} linuxdeploy`,
      ).stdout.trim();
      assert.equal(main, join(app, "usr/bin/OpenHuman"));
      assert.ok(
        validateLinuxdeploy(app),
        "a well-formed linuxdeploy AppDir must validate",
      );
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  },
);

test("still classifies and resolves a sharun AppDir", SKIP, () => {
  const { root, app } = makeSharunAppDir();
  try {
    assert.equal(layoutOf(app), "sharun");
    const main = bash(
      `appdir_main_binary ${JSON.stringify(app)} sharun`,
    ).stdout.trim();
    assert.equal(main, join(app, "shared/bin/OpenHuman"));
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test(
  "an unrecognised layout is reported as unknown, not silently accepted",
  SKIP,
  () => {
    const root = fs.mkdtempSync(join(os.tmpdir(), "openhuman-appdir-unk-"));
    try {
      fs.mkdirSync(join(root, "OpenHuman.AppDir/random"), { recursive: true });
      fs.writeFileSync(join(root, "OpenHuman.AppDir/random/thing"), "");
      assert.equal(layoutOf(join(root, "OpenHuman.AppDir")), "unknown");
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  },
);

// The x86_64 smoke path installs a per-executable AppArmor profile, and its
// target was hardcoded to the sharun path. Static validation passed while the
// smoke died on "AppArmor target is not executable" - the same defect as #5606
// one layer down, caught only because the fork demo ran the full matrix.
test(
  "the AppArmor smoke target is resolved per layout, not hardcoded",
  SKIP,
  () => {
    const { root, app } = makeLinuxdeployAppDir();
    const binDir = fs.mkdtempSync(join(os.tmpdir(), "openhuman-appdir-stub-"));
    try {
      // Stub the privileged tools so the test exercises target resolution only.
      for (const name of ["sudo", "apparmor_parser"]) {
        script(join(binDir, name), "#!/bin/sh\nexit 0\n");
      }
      const profile = join(root, "smoke.profile");
      const result = spawnSync(
        "bash",
        [
          "-c",
          `source ${JSON.stringify(VALIDATOR)} >/dev/null 2>&1\n` +
            `install_smoke_userns_profile ${JSON.stringify(app)} ${JSON.stringify(profile)}`,
        ],
        {
          encoding: "utf8",
          env: { ...process.env, PATH: `${binDir}:${process.env.PATH ?? ""}` },
        },
      );

      assert.doesNotMatch(
        result.stderr,
        /AppArmor target is not executable/,
        "the AppArmor target must resolve for a linuxdeploy AppDir",
      );
      assert.equal(result.status, 0, result.stderr);
      // The profile must confine the real binary, not the sharun path.
      const written = fs.readFileSync(profile, "utf8");
      assert.match(written, /usr\/bin\/OpenHuman/);
      assert.doesNotMatch(written, /shared\/bin/);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
      fs.rmSync(binDir, { recursive: true, force: true });
    }
  },
);

// Failure paths: each mutation must be rejected. Without these the validator
// could pass everything and nobody would notice until a broken bundle shipped.
const MUTATIONS = [
  [
    "AppRun does not hand off to AppRun.wrapped",
    (a) => script(join(a, "AppRun"), "#!/bin/sh\nexec /bin/true\n"),
  ],
  [
    "AppRun.wrapped is not an ELF",
    (a) => fs.writeFileSync(join(a, "AppRun.wrapped"), "nope"),
  ],
  [
    "AppRun is an ELF rather than the wrapper script",
    (a) => elf(join(a, "AppRun")),
  ],
  ["AppRun is not executable", (a) => fs.chmodSync(join(a, "AppRun"), 0o644)],
  [
    "an apprun hook is not sourced by AppRun",
    (a) => script(join(a, "apprun-hooks/orphan.sh"), "#!/bin/sh\n"),
  ],
  [
    "the desktop entry has no Exec=",
    (a) => {
      const p = join(a, "usr/share/applications/OpenHuman.desktop");
      fs.writeFileSync(p, fs.readFileSync(p, "utf8").replace(/^Exec=.*$/m, ""));
    },
  ],
  [
    "there are two desktop entries",
    (a) =>
      fs.copyFileSync(
        join(a, "usr/share/applications/OpenHuman.desktop"),
        join(a, "usr/share/applications/Other.desktop"),
      ),
  ],
  [
    "there is no desktop entry",
    (a) => fs.rmSync(join(a, "usr/share/applications/OpenHuman.desktop")),
  ],
  [
    "usr/lib contains no shared libraries",
    (a) => {
      for (const f of fs.readdirSync(join(a, "usr/lib")))
        fs.rmSync(join(a, "usr/lib", f));
    },
  ],
  [
    ".DirIcon dangles",
    (a) =>
      fs.rmSync(
        join(a, "usr/share/icons/hicolor/256x256@2/apps/OpenHuman.png"),
      ),
  ],
];

for (const [name, mutate] of MUTATIONS) {
  test(`rejects an AppDir where ${name}`, SKIP, () => {
    const { root, app } = makeLinuxdeployAppDir();
    try {
      mutate(app);
      assert.ok(
        !validateLinuxdeploy(app),
        `expected validation to fail when ${name}`,
      );
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });
}
