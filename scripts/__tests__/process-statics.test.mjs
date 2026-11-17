import assert from "node:assert/strict";
import test from "node:test";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  declarations,
  reviewChanges,
  checkRepository,
} from "../ci/check-process-statics.mjs";

const file = "crates/openhuman-core/src/example/ops.rs";
const source = "static CACHE: OnceLock<Mutex<Map>> = OnceLock::new();";
const entry = {
  file,
  name: "CACHE",
  type: "OnceLock<Mutex<Map>>",
  count: 1,
  category: "B",
  reason: "Keys include the authenticated profile.",
  verification: "example/ops_tests.rs: profiles_cannot_share_cache",
};
const scan = (text) => declarations(text, file);

function withRepository(initialSource, approvals, callback) {
  const root = mkdtempSync(join(tmpdir(), "process-statics-review-"));
  const git = (...args) =>
    execFileSync("git", args, { cwd: root, stdio: "pipe" }).toString().trim();
  try {
    mkdirSync(join(root, "crates/openhuman-core/src/example"), {
      recursive: true,
    });
    mkdirSync(join(root, "scripts/ci"), { recursive: true });
    writeFileSync(join(root, file), initialSource);
    writeFileSync(
      join(root, "scripts/ci/process-statics.json"),
      JSON.stringify(approvals),
    );
    git("init");
    git("add", ".");
    git(
      "-c",
      "user.name=Test",
      "-c",
      "user.email=test@example.invalid",
      "-c",
      "core.hooksPath=/dev/null",
      "commit",
      "-m",
      "base",
    );
    return callback(root, git, git("rev-parse", "HEAD"));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test("previous ownership records cannot be removed while the declaration survives", () => {
  withRepository(source, [entry], (root, _git, base) => {
    writeFileSync(join(root, "scripts/ci/process-statics.json"), "[]");
    assert.equal(checkRepository(root, base).errors.length > 0, true);
  });
});

test("non-ASCII and newline filenames cannot bypass the tracked source scan", () => {
  withRepository("pub fn example() {}", [], (root, git, base) => {
    for (const name of ["缓存.rs", "line\nbreak.rs"]) {
      const path = "crates/openhuman-core/src/" + name;
      writeFileSync(join(root, path), source);
      git("add", path);
    }
    assert.equal(checkRepository(root, base).errors.length, 2);
    git(
      "-c",
      "user.name=Test",
      "-c",
      "user.email=test@example.invalid",
      "-c",
      "core.hooksPath=/dev/null",
      "commit",
      "-m",
      "legacy paths",
    );
    assert.deepEqual(
      checkRepository(root, git("rev-parse", "HEAD")).errors,
      [],
    );
  });
});

test("prior ownership records can be removed when their declaration disappears", () => {
  withRepository(source, [entry], (root, _git, base) => {
    writeFileSync(join(root, file), "pub fn example() {}");
    writeFileSync(join(root, "scripts/ci/process-statics.json"), "[]");
    assert.deepEqual(checkRepository(root, base).errors, []);
  });
});

test("ABI literals remain part of the declared type signature", () => {
  const before = scan('static CALLBACK: extern "C" fn() = x;');
  const after = scan('static CALLBACK: extern "C-unwind" fn() = x;');
  assert.notEqual(before[0].type, after[0].type);
  assert.equal(reviewChanges(before, after, []).length, 1);
});

test("whitespace inside type literals remains significant", () => {
  const before = scan('static X: [u8; "a b".len()] = x;');
  const after = scan('static X: [u8; "ab".len()] = x;');
  assert.notEqual(before[0].type, after[0].type);
  assert.equal(reviewChanges(before, after, []).length, 1);
});

test("finds multiline, qualified, mutable and lazy_static declarations", () => {
  assert.deepEqual(
    scan(
      "static CACHE:\n std::sync::OnceLock<Mutex<Map>> = x;\nstatic mut FLAG: bool = false;\nlazy_static! { static ref OTHER: Alias = x; }",
    ).map(({ name, type }) => ({ name, type })),
    [
      { name: "CACHE", type: "std::sync::OnceLock<Mutex<Map>>" },
      { name: "FLAG", type: "mut bool" },
      { name: "OTHER", type: "ref Alias" },
    ],
  );
});
test("ignores comments and quoted Rust fixtures", () => {
  assert.deepEqual(
    scan(
      "// " +
        source +
        "\n/* outer /* " +
        source +
        ' */ */\nconst S: &str = r###"' +
        source +
        '"###;',
    ),
    [],
  );
});
test("unreviewed new statics fail even when their type hides a mutex", () => {
  assert.equal(
    reviewChanges([], scan("static CACHE: Alias = x;"), []).length,
    1,
  );
});
test("unchanged existing statics need no retrospective approval", () => {
  assert.deepEqual(reviewChanges(scan(source), scan(source), []), []);
});
test("a new declaration passes with an exact ownership record", () => {
  assert.deepEqual(reviewChanges([], scan(source), [entry]), []);
});
test("type changes and duplicate declarations require a matching record", () => {
  assert.equal(
    reviewChanges(scan(source), scan(source.replace("Map", "Other")), [entry])
      .length > 0,
    true,
  );
  assert.equal(
    reviewChanges(
      scan(source),
      scan(source + "\nmod nested { " + source + " }"),
      [entry],
    ).length > 0,
    true,
  );
});
test("removed or mismatched approvals fail", () => {
  assert.equal(reviewChanges([], [], [entry]).length > 0, true);
  assert.equal(
    reviewChanges([], scan(source), [{ ...entry, count: 2 }]).length > 0,
    true,
  );
});
test("classification, reason and verification are mandatory", () => {
  for (const patch of [
    { category: "Z" },
    { reason: "" },
    { verification: "" },
  ]) {
    assert.equal(
      reviewChanges([], scan(source), [{ ...entry, ...patch }]).length > 0,
      true,
    );
  }
});
test("duplicate approval records fail", () => {
  assert.equal(
    reviewChanges([], scan(source), [entry, entry]).length > 0,
    true,
  );
});
test("lifetimes and associated types are not declarations", () => {
  assert.deepEqual(
    scan("fn value() -> &'static tinyagents_harness::Value { todo!() }"),
    [],
  );
});
test("raw and Unicode identifiers cannot bypass ownership review", () => {
  const current = scan("static r#type: Alias = x; static 缓存: Alias = x;");
  assert.deepEqual(
    current.map(({ name }) => name),
    ["r#type", "缓存"],
  );
  assert.equal(reviewChanges([], current, []).length, 2);
});
test("const generic equality and function arrows do not terminate a type", () => {
  assert.equal(
    scan("static X: Wrapper<{1 == 1}, fn() -> usize> = x;")[0].type,
    "Wrapper<{1==1},fn()->usize>",
  );
});
test("comparisons and shifts inside const blocks are not generic brackets", () => {
  assert.equal(
    scan("static X: Wrapper<{1 << 2}, {1 < 2}> = x;")[0].type,
    "Wrapper<{1<<2},{1<2}>",
  );
});
test("array length expressions do not treat shifts as generic brackets", () => {
  assert.equal(
    scan("static X: [Option<u8>; 1 << 2] = x;")[0].type,
    "[Option<u8>;1<<2]",
  );
});
test("swapping same-named static types between modules requires ownership review", () => {
  const before = scan(
    "mod user { static CACHE: UserCache = x; } mod infra { static CACHE: InfraCache = x; }",
  );
  const after = scan(
    "mod user { static CACHE: InfraCache = x; } mod infra { static CACHE: UserCache = x; }",
  );
  assert.equal(reviewChanges(before, after, []).length, 2);
});
test("module ownership records survive line and module-order changes but cannot cover another module", () => {
  const before = scan(`mod outer { mod user { ${source} } }`);
  const approval = { ...entry, scope: "outer::user" };
  assert.deepEqual(reviewChanges([], before, [approval]), []);
  assert.deepEqual(
    reviewChanges(
      before,
      scan(`mod other {}\n\nmod outer { mod user { ${source} } }`),
      [approval],
    ),
    [],
  );
  assert.equal(
    reviewChanges(before, scan(`mod outer { mod infra { ${source} } }`), [
      approval,
    ]).length > 0,
    true,
  );
});
test("type macros preserve comparison tokens in parentheses and square token trees", () => {
  for (const type of [
    "ty!(1 < 2)",
    "ty![1 > 2]",
    "Wrapper<ty!(1 << 2), Option<u8>>",
  ]) {
    assert.equal(
      scan(`static X: ${type} = x;`)[0].type,
      type.replaceAll(" ", ""),
    );
  }
});
test("repository scan reads the merge base, enforces records, and fails without history", () => {
  const root = mkdtempSync(join(tmpdir(), "process-statics-"));
  const git = (...args) =>
    execFileSync("git", args, {
      cwd: root,
      stdio: "pipe",
      env: { ...process.env, GIT_CONFIG_COUNT: "0" },
    })
      .toString()
      .trim();
  try {
    mkdirSync(join(root, "crates/openhuman-core/src/example"), {
      recursive: true,
    });
    mkdirSync(join(root, "scripts/ci"), { recursive: true });
    writeFileSync(join(root, file), "pub fn example() {}");
    writeFileSync(join(root, "scripts/ci/process-statics.json"), "[]");
    git("init");
    git("add", ".");
    git(
      "-c",
      "user.name=Test",
      "-c",
      "user.email=test@example.invalid",
      "-c",
      "core.hooksPath=/dev/null",
      "commit",
      "-m",
      "base",
    );
    const base = git("rev-parse", "HEAD");
    writeFileSync(join(root, file), source);
    assert.equal(checkRepository(root, base).errors.length, 1);
    writeFileSync(
      join(root, "scripts/ci/process-statics.json"),
      JSON.stringify([entry]),
    );
    assert.deepEqual(checkRepository(root, base).errors, []);
    assert.throws(
      () => checkRepository(root, "missing-base"),
      /fetch the base/,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

 test("type macro token boundaries cannot reuse an earlier ownership signature", () => {
  for (const [beforeTokens, afterTokens] of [
    ["a b", "ab"], ["a/**/b", "ab"], ["1 u8", "1u8"],
    ['b "x"', 'b"x"'], ["> >", ">>"], ["r #ident", "r#ident"],
  ]) {
    const before = scan(`static X: ty!(${beforeTokens}) = x;`);
    const after = scan(`static X: ty!(${afterTokens}) = x;`);
    assert.notEqual(before[0].type, after[0].type, beforeTokens);
    assert.equal(reviewChanges(before, after, []).length, 1);
  }
});

test("literal static declarations in macro templates require source ownership review", () => {
  const before = scan("pub fn example() {}");
  const after = scan("macro_rules! make_cache { () => { static CACHE: OnceLock<Map> = x; }; }");
  assert.equal(after.length, 1);
  assert.equal(reviewChanges(before, after, []).length, 1);
});

test("numeric and supplementary Unicode macro token boundaries remain distinct", () => {
  for (const [left, right] of [["1 . 0", "1.0"], ["𐐀 𐐁", "𐐀𐐁"]]) {
    const before = scan(`static X: ty!(${left}) = x;`);
    const after = scan(`static X: ty!(${right}) = x;`);
    assert.notEqual(before[0].type, after[0].type, left);
    assert.equal(reviewChanges(before, after, []).length, 1);
  }
});

test("macro punctuation jointness cannot change without ownership review", () => {
  for (const [left, right] of [["a.. =b", "a..=b"], [".. =", "..="], ["# $", "#$"], ["1e +2", "1e+2"]]) {
    const before = scan(`static X: ty!(${left}) = x;`);
    const after = scan(`static X: ty!(${right}) = x;`);
    assert.notEqual(before[0].type, after[0].type, left);
    assert.equal(reviewChanges(before, after, []).length, 1);
  }
});

test("numeric token boundaries after exponent signs and before suffixes remain distinct", () => {
  for (const [left, right] of [["1e+ 2", "1e+2"], ["1e- 2", "1e-2"], ["1. e2", "1.e2"]]) {
    const before = scan(`static X: ty!(${left}) = x;`);
    const after = scan(`static X: ty!(${right}) = x;`);
    assert.notEqual(before[0].type, after[0].type, left);
    assert.equal(reviewChanges(before, after, []).length, 1);
  }
});
