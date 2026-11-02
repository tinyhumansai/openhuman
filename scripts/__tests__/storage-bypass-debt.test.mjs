import assert from "node:assert/strict";
import test from "node:test";
import {
  mkdtempSync,
  rmSync,
  mkdirSync,
  writeFileSync,
  symlinkSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync, spawnSync } from "node:child_process";
import {
  validateCoverageDebt,
  validateAllowances,
  historicalFindings,
} from "../ci/storage-bypass-debt.mjs";
import {
  compare,
  scan,
  CURATED_VENDOR_ROOTS,
  collectFindings,
} from "../ci/check-storage-bypass.mjs";
const row = {
  rule: "json-write",
  path: "crates/openhuman-embed/src/state.rs",
  text: "tokio::fs::write(path, bytes);",
  occurrence: 1,
  classification: "unmigrated",
  reason: "Existing native backend migration debt at this exact source site.",
  issue: "https://github.com/tinyhumansai/openhuman/issues/7428",
};
const history = { baseline: [], extension: null, findings: [row] };
test("initial extension admits only exact existing canonical coverage", () => {
  assert.deepEqual(validateCoverageDebt([row], [], [row], history), []);
  const added = { ...row, path: "crates/openhuman-embed/src/new.rs" };
  assert.match(
    validateCoverageDebt([added], [], [added], history).join("\n"),
    /branch-added writer/,
  );
});
test("moving or copying a recorded writer cannot reuse its identity", () => {
  const moved = { ...row, path: "crates/openhuman-rpc/src/state.rs" };
  assert.equal(compare([moved], [row]).added.length, 1);
  const copied = { ...row, occurrence: 2 };
  assert.equal(compare([row, copied], [row]).added.length, 1);
});
test("extension records may shrink but never grow after establishment", () => {
  const old = { ...history, extension: [row] };
  assert.deepEqual(validateCoverageDebt([], [], [], old), []);
  const added = { ...row, text: "tokio::fs::write(other, bytes);" };
  assert.match(
    validateCoverageDebt([row, added], [], [row, added], old).join("\n"),
    /Extension growth forbidden/,
  );
});
test("removing a debt record does not grandfather a surviving writer", () => {
  assert.equal(compare([row], []).added.length, 1);
});
test("original core debt cannot move into the extension or grow", () => {
  const core = { ...row, path: "crates/openhuman-core/src/state.rs" };
  assert.match(
    validateCoverageDebt([core], [], [core], history).join("\n"),
    /cannot conceal original core debt/,
  );
  assert.match(
    validateCoverageDebt([core], [core], [], history).join("\n"),
    /Core baseline growth forbidden/,
  );
});
test("reasons, exact occurrence and issue links are mandatory", () => {
  for (const bad of [
    { reason: "" },
    { issue: "" },
    { classification: "approved" },
    { occurrence: 2 },
  ]) {
    assert.ok(
      validateCoverageDebt([row], [], [{ ...row, ...bad }], history).length,
    );
  }
});
test("an initial allowance cannot exempt a newly introduced branch writer", () => {
  const record = { ...row, site: row.text };
  assert.match(
    validateAllowances([row], [record], {
      rawFindings: [],
      allowances: null,
    }).join("\n"),
    /branch-added writer/,
  );
  assert.deepEqual(
    validateAllowances([row], [record], {
      rawFindings: [row],
      allowances: null,
    }),
    [],
  );
});
test("later allowances cannot grow or excuse a copied site", () => {
  const record = { ...row, site: row.text };
  const copy = { ...record, path: "crates/openhuman-embed/src/copy.rs" };
  assert.match(
    validateAllowances([{ ...row, path: copy.path }], [copy], {
      rawFindings: [row],
      allowances: [record],
    }).join("\n"),
    /growth forbidden/,
  );
});
test("curated vendor coverage excludes native drivers but not host adapter writes", () => {
  assert.ok(
    CURATED_VENDOR_ROOTS.every((p) => !p.includes("tinystoragedrivers")),
  );
  assert.ok(CURATED_VENDOR_ROOTS.some((p) => p.includes("tinymcp")));
  assert.ok(CURATED_VENDOR_ROOTS.some((p) => p.includes("run_ledger")));
  assert.equal(
    scan(
      "crates/openhuman-embed/src/adapter.rs",
      "tokio::fs::write(path, bytes);",
    ).length,
    1,
  );
});
test("new serializer and helper rules ignore quoted and commented fixture text", () => {
  assert.deepEqual(
    scan(
      "x.rs",
      'let fixture = "serde_json::to_writer(file, value);";\n/* fn write_atomic(path) {} */',
    ),
    [],
  );
});

test("missing canonical base history fails closed", () => {
  const root = mkdtempSync(join(tmpdir(), "storage-history-"));
  try {
    execFileSync("git", ["init"], { cwd: root, stdio: "ignore" });
    assert.throws(() =>
      historicalFindings(root, {
        scan,
        withoutAllowed: (_, x) => x,
        isTestFile: () => false,
        CURATED_VENDOR_ROOTS: [],
      }),
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
test("quoted database opener examples do not count as host persistence", () => {
  assert.deepEqual(
    scan(
      "crates/openhuman-embed/src/example.rs",
      'let example = r#"OtherStore::open(root.join("cache.db"))"#;',
    ),
    [],
  );
});

test("duplicate original core identities cannot inflate the retained baseline", () => {
  const core = { ...row, path: "crates/openhuman-core/src/state.rs" };
  assert.match(
    validateCoverageDebt([core], [core, core], [], {
      baseline: [core],
      extension: [],
      findings: [core],
    }).join("\n"),
    /Duplicate core baseline record/,
  );
});

test("real checkouts refuse missing curated vendor source instead of shrinking debt", async () => {
  const root = mkdtempSync(join(tmpdir(), "storage-curated-source-"));
  try {
    execFileSync("git", ["init"], { cwd: root, stdio: "ignore" });
    mkdirSync(join(root, "crates"));
    await assert.rejects(collectFindings(root), /Missing curated source/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("present but empty curated vendor directories cannot appear as debt shrinkage", async () => {
  const root = mkdtempSync(join(tmpdir(), "storage-empty-curated-"));
  try {
    execFileSync("git", ["init"], { cwd: root, stdio: "ignore" });
    mkdirSync(join(root, "crates"));
    for (const path of CURATED_VENDOR_ROOTS) {
      if (path.endsWith(".rs")) {
        mkdirSync(join(root, path, ".."), { recursive: true });
        writeFileSync(join(root, path), "// no writer\n");
      } else mkdirSync(join(root, path), { recursive: true });
    }
    await assert.rejects(collectFindings(root), /Empty curated source/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("symlinked real checkouts still refuse missing curated source", async () => {
  const fixture = mkdtempSync(join(tmpdir(), "storage-symlink-source-"));
  const root = join(fixture, "checkout");
  const linked = join(fixture, "linked");
  try {
    mkdirSync(root);
    execFileSync("git", ["init"], { cwd: root, stdio: "ignore" });
    mkdirSync(join(root, "crates"));
    symlinkSync(root, linked, "junction");
    await assert.rejects(collectFindings(linked), /Missing curated source/);
  } finally {
    rmSync(fixture, { recursive: true, force: true });
  }
});

test("symlinked --root checkouts cannot bypass required canonical history", () => {
  const fixture = mkdtempSync(join(tmpdir(), "storage-symlink-history-"));
  const root = join(fixture, "checkout");
  const linked = join(fixture, "linked");
  try {
    mkdirSync(root);
    execFileSync("git", ["init"], { cwd: root, stdio: "ignore" });
    mkdirSync(join(root, "crates"));
    for (const path of CURATED_VENDOR_ROOTS) {
      if (path.endsWith(".rs")) {
        mkdirSync(join(root, path, ".."), { recursive: true });
        writeFileSync(join(root, path), "// no writer\n");
      } else {
        mkdirSync(join(root, path), { recursive: true });
        writeFileSync(join(root, path, "store.rs"), "// no writer\n");
      }
    }
    mkdirSync(join(root, "scripts", "ci"), { recursive: true });
    writeFileSync(
      join(root, "scripts", "ci", "storage-bypass-baseline.json"),
      "[]\n",
    );
    symlinkSync(root, linked, "junction");
    const result = spawnSync(
      process.execPath,
      [
        fileURLToPath(
          new URL("../ci/check-storage-bypass.mjs", import.meta.url),
        ),
        "--root",
        linked,
      ],
      { encoding: "utf8" },
    );
    assert.equal(result.status, 1, result.stdout + result.stderr);
    assert.match(result.stderr, /merge-base|origin\/main|upstream\/main/);
  } finally {
    rmSync(fixture, { recursive: true, force: true });
  }
});

test("new-based file stores are findings without matching memory stores", () => {
  const findings = scan(
    "crates/openhuman-core/src/catalog.rs",
    [
      "let cache = FileCatalogStore::new(dir);",
      "let cache = tinyskills::FileCatalogStore::new(dir);",
      "let memory = MemoryCatalogStore::new();",
    ].join("\n"),
  );
  assert.deepEqual(
    findings.map((f) => f.rule),
    ["file-store-new", "file-store-new"],
  );
});

for (const [name, source] of [
  ["task stores", "vendor/tinyagents/crates/tinyagents-tasks/src/store.rs"],
  [
    "catalog store",
    "vendor/tinyskills/crates/tinyskills/src/registry/store.rs",
  ],
]) {
  test(`${name} owning writers are included in curated coverage`, async () => {
    const root = mkdtempSync(join(tmpdir(), "storage-owner-coverage-"));
    try {
      mkdirSync(join(root, "crates"));
      mkdirSync(join(root, source, ".."), { recursive: true });
      writeFileSync(
        join(root, source),
        "pub fn persist() { std::fs::write(path, bytes); }\n",
      );
      const findings = await collectFindings(root);
      assert.equal(findings.length, 1);
      assert.equal(findings[0].path, source);
      assert.equal(findings[0].rule, "json-write");
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
}

test("host calls to the JSONL task opener cannot hide their persistent path", () => {
  const findings = scan(
    "crates/openhuman-core/src/tasks.rs",
    "let store = tinyagents::open_jsonl_task_store_or_memory(path, None);",
  );
  assert.equal(findings.length, 1);
  assert.equal(findings[0].rule, "task-store-open");
});

test("current thread and turn-state host facades remain curated after storage migration", () => {
  assert.ok(
    CURATED_VENDOR_ROOTS.includes(
      "vendor/tinyagents/crates/tinyagents-session/src/threads/mod.rs",
    ),
  );
  assert.ok(
    CURATED_VENDOR_ROOTS.includes(
      "vendor/tinyagents/crates/tinyagents-session/src/turn_state/mirror",
    ),
  );
});

test("a Git-recorded move preserves only the original writer and allowance", () => {
  const root = mkdtempSync(join(tmpdir(), "storage-debt-move-"));
  const git = (...args) => execFileSync("git", args, {
    cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"],
  });
  const oldPath = "crates/openhuman-core/src/old/state.rs";
  const newPath = "crates/openhuman-core/src/new/state.rs";
  const source = Array.from({ length: 30 }, (_, i) => "// original comment " + i).join("\n") +
    "\ntokio::fs::write(path, bytes);\n";
  const old = { ...row, path: oldPath };
  const moved = { ...old, path: newPath };
  const scanner = { scan, withoutAllowed: (_, x) => x, isTestFile: () => false, CURATED_VENDOR_ROOTS: [] };
  try {
    git("init", "-b", "main");
    git("config", "user.name", "Test");
    git("config", "user.email", "test@example.invalid");
    mkdirSync(join(root, oldPath, ".."), { recursive: true });
    mkdirSync(join(root, "scripts/ci"), { recursive: true });
    writeFileSync(join(root, oldPath), source);
    writeFileSync(join(root, "scripts/ci/storage-bypass-baseline.json"), JSON.stringify([old]));
    writeFileSync(join(root, "scripts/ci/storage-bypass-extension.json"), "[]");
    writeFileSync(join(root, "scripts/ci/storage-bypass-allow-reasons.json"), JSON.stringify([{ ...old, site: old.text }]));
    git("add", ".");
    git("commit", "-m", "original writer");
    git("update-ref", "refs/remotes/upstream/main", "HEAD");
    // A pull request run sets GITHUB_BASE_REF, which selects origin/<base>.
    git("update-ref", `refs/remotes/origin/${process.env.GITHUB_BASE_REF || "main"}`, "HEAD");
    git("switch", "-c", "move");
    mkdirSync(join(root, newPath, ".."), { recursive: true });
    git("mv", oldPath, newPath);
    writeFileSync(join(root, newPath), source + "tokio::fs::write(other, bytes);\n");
    git("add", ".");
    const previous = historicalFindings(root, scanner);
    assert.deepEqual(validateCoverageDebt([moved], [moved], [], previous), []);
    assert.deepEqual(validateAllowances([moved], [{ ...moved, site: moved.text }], previous), []);
    const added = { ...moved, text: "tokio::fs::write(other, bytes);" };
    assert.match(validateCoverageDebt([moved, added], [moved, added], [], previous).join("\n"), /growth forbidden/);
    assert.match(validateAllowances([moved, added], [{ ...added, site: added.text }], previous).join("\n"), /growth forbidden/);
    const copied = { ...moved, path: "crates/openhuman-core/src/copied.rs" };
    assert.match(validateCoverageDebt([copied], [copied], [], previous).join("\n"), /growth forbidden/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a unique unchanged writer may move during a module split", () => {
  const old = {
    rule: "json-write",
    path: "crates/openhuman-core/src/credentials/profiles.rs",
    text: "let mut file = OpenOptions::new()",
    occurrence: 1,
  };
  const moved = {
    ...old,
    path: "crates/openhuman-core/src/credentials/profiles/persistence.rs",
  };
  const history = {
    baseline: [old],
    extension: [],
    findings: [old],
    changedPaths: new Set([moved.path]),
  };

  assert.deepEqual(validateCoverageDebt([moved], [moved], [], history), []);

  const added = { ...moved, text: "fs::write(other, bytes)" };
  assert.match(
    validateCoverageDebt([moved, added], [moved, added], [], history).join("\n"),
    /Core baseline growth forbidden/,
  );
});

test("a unique extension writer survives a heavy rewrite move Git misses", () => {
  const root = mkdtempSync(join(tmpdir(), "storage-debt-rewrite-move-"));
  const git = (...args) => execFileSync("git", args, {
    cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"],
  });
  const oldPath = "crates/openhuman-embed/src/credentials/profiles.rs";
  const newPath = "crates/openhuman-embed/src/profiles.rs";
  const source = "tokio::fs::write(path, bytes);\n";
  const old = { ...row, path: oldPath };
  const moved = { ...old, path: newPath };
  const scanner = { scan, withoutAllowed: (_, x) => x, isTestFile: () => false, CURATED_VENDOR_ROOTS: [] };
  try {
    git("init", "-b", "main");
    git("config", "user.name", "Test");
    git("config", "user.email", "test@example.invalid");
    mkdirSync(join(root, oldPath, ".."), { recursive: true });
    mkdirSync(join(root, "scripts/ci"), { recursive: true });
    writeFileSync(join(root, oldPath), source);
    writeFileSync(join(root, "scripts/ci/storage-bypass-baseline.json"), "[]\n");
    writeFileSync(join(root, "scripts/ci/storage-bypass-extension.json"), JSON.stringify([old]));
    git("add", ".");
    git("commit", "-m", "original extension writer");
    git("update-ref", "refs/remotes/upstream/main", "HEAD");
    git("update-ref", `refs/remotes/origin/${process.env.GITHUB_BASE_REF || "main"}`, "HEAD");
    git("switch", "-c", "move");
    rmSync(join(root, oldPath));
    writeFileSync(join(root, newPath), [
      "// substantially rewritten module layout",
      "// Git should see a delete and add rather than a rename.",
      "pub fn persist_profile() {",
      source.trimEnd(),
      "}",
      "// additional unrelated implementation lines",
    ].join("\n") + "\n");
    git("add", "-A");
    const changes = git("diff", "--cached", "--name-status", "--find-renames=50%", "HEAD");
    assert.match(changes, /^D\t/);
    assert.match(changes, /\nA\t/);
    assert.doesNotMatch(changes, /^R\d*\t/m);

    const previous = historicalFindings(root, scanner);
    assert.deepEqual(validateCoverageDebt([moved], [], [moved], previous), []);
    const copied = { ...moved, path: "crates/openhuman-embed/src/copied.rs" };
    assert.match(
      validateCoverageDebt([old, copied], [], [old, copied], previous).join("\n"),
      /Extension growth forbidden/,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
