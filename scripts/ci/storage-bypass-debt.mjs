import { execFileSync } from "node:child_process";

const CORE = "crates/openhuman-core/src/";
const BASELINE = "scripts/ci/storage-bypass-baseline.json";
const EXTENSION = "scripts/ci/storage-bypass-extension.json";
const key = (x) => JSON.stringify([x.rule, x.path, x.text, x.occurrence]);
const original = (x) =>
  x.path.startsWith(CORE) && ["sqlite-open", "json-write"].includes(x.rule);

/** Exact existing-debt records never authorize another site or later growth. */
export function validateCoverageDebt(findings, baseline, extension, history) {
  if (!Array.isArray(baseline) || !Array.isArray(extension))
    return ["Storage debt manifests must be arrays"];
  const errors = [];
  const actual = new Set(findings.map(key));
  const seen = new Set();
  const coreSeen = new Set();
  for (const record of baseline) {
    const id = key(record);
    if (coreSeen.has(id))
      errors.push("Duplicate core baseline record: " + record.path);
    coreSeen.add(id);
    if (!original(record))
      errors.push(
        `Core baseline cannot accept extended coverage: ${record.path}`,
      );
  }
  for (const record of extension) {
    const id = key(record);
    if (seen.has(id)) errors.push(`Duplicate extension record: ${record.path}`);
    seen.add(id);
    if (original(record))
      errors.push(
        `Extension cannot conceal original core debt: ${record.path}`,
      );
    if (!actual.has(id))
      errors.push(`Stale extension record: ${record.path}: ${record.text}`);
    if (
      ![
        "classic",
        "machine",
        "scratch",
        "external-export",
        "read-only",
        "unmigrated",
      ].includes(record.classification) ||
      typeof record.reason !== "string" ||
      record.reason.trim().length < 20 ||
      !/^https:\/\/github\.com\/tinyhumansai\/openhuman\/issues\/\d+$/.test(
        record.issue ?? "",
      )
    ) {
      errors.push(`Incomplete audited extension record: ${record.path}`);
    }
  }
  if (history) {
    const oldCore = new Set(history.baseline.map(key));
    // A module split can move one existing writer while leaving its original
    // file in place, so Git does not report a whole-file rename. Carry that
    // debt forward only when the old path no longer has the site and exactly
    // one current site has the same rule, source text, and occurrence.
    for (const record of baseline) {
      if (oldCore.has(key(record))) continue;
      const sameSite = (candidate) =>
        candidate.rule === record.rule &&
        candidate.text === record.text &&
        candidate.occurrence === record.occurrence;
      const previous = history.baseline.filter(
        (candidate) => sameSite(candidate) && !actual.has(key(candidate)),
      );
      const current = findings.filter(
        (candidate) =>
          sameSite(candidate) && history.changedPaths?.has(candidate.path),
      );
      if (
        previous.length === 1 &&
        current.length === 1 &&
        history.changedPaths?.has(record.path) &&
        previous[0].path !== record.path &&
        current[0].path === record.path
      ) {
        oldCore.add(key(record));
      }
    }
    for (const record of baseline)
      if (!oldCore.has(key(record)))
        errors.push(
          `Core baseline growth forbidden: ${record.path}: ${record.text}`,
        );
    const oldExtension =
      history.extension === null ? null : new Set(history.extension.map(key));
    const oldSites = new Set(history.findings.map(key));
    const movedExtension = new Set();
    if (oldExtension) {
      for (const record of extension) {
        if (oldExtension.has(key(record))) continue;
        const sameSite = (candidate) =>
          candidate.rule === record.rule &&
          candidate.text === record.text &&
          candidate.occurrence === record.occurrence;
        const previous = history.extension.filter(
          (candidate) =>
            sameSite(candidate) &&
            !actual.has(key(candidate)) &&
            history.changedPaths?.has(candidate.path),
        );
        const current = findings.filter(
          (candidate) =>
            sameSite(candidate) &&
            history.changedPaths?.has(candidate.path),
        );
        if (
          previous.length === 1 &&
          current.length === 1 &&
          previous[0].path !== record.path &&
          current[0].path === record.path &&
          history.changedPaths?.has(record.path)
        ) {
          movedExtension.add(key(record));
        }
      }
    }
    for (const record of extension) {
      if (
        oldExtension &&
        !oldExtension.has(key(record)) &&
        !movedExtension.has(key(record))
      )
        errors.push(
          `Extension growth forbidden: ${record.path}: ${record.text}`,
        );
      if (!oldExtension && !oldSites.has(key(record)))
        errors.push(
          `Initial extension cannot approve a branch-added writer: ${record.path}: ${record.text}`,
        );
    }
  }
  return errors;
}

/** Re-scan canonical merge-base source, including the recorded curated gitlinks. */
export function historicalFindings(root, scanner) {
  const git = (args, cwd = root) =>
    execFileSync("git", args, {
      cwd,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  let ref = process.env.GITHUB_BASE_REF
    ? `origin/${process.env.GITHUB_BASE_REF}`
    : "upstream/main";
  if (!process.env.GITHUB_BASE_REF) {
    try {
      git(["rev-parse", "--verify", ref]);
    } catch {
      ref = "origin/main";
    }
  }
  const base = git(["merge-base", ref, "HEAD"]).trim();
  // Git must identify a rename (the original path disappears). Copies and
  // new files retain their own identity. Only the path changes: exact writer
  // text and occurrence remain part of every debt/allowance key.
  const changes = git([
    "diff", "--name-status", "-z", "--find-renames=50%", base, "--", "crates",
  ]).split("\0");
  const renamed = new Map();
  const changedPaths = new Set();
  for (let at = 0; at < changes.length && changes[at];) {
    const status = changes[at++];
    const from = changes[at++];
    changedPaths.add(from);
    if (/^[RC]/.test(status)) {
      const to = changes[at++];
      changedPaths.add(to);
      if (status.startsWith("R")) renamed.set(from, to);
    }
  }
  const relocated = (record) => renamed.has(record.path)
    ? { ...record, path: renamed.get(record.path) }
    : record;
  const files = git([
    "ls-tree",
    "-r",
    "-z",
    "--name-only",
    base,
    "--",
    "crates",
  ])
    .split("\0")
    .filter((p) => /^crates\/[^/]+\/src\/.*\.rs$/.test(p));
  const findings = [];
  const rawFindings = [];
  const add = (path, source) => {
    if (!scanner.isTestFile(path)) {
      const destination = renamed.get(path) ?? path;
      const raw = scanner.scan(destination, source);
      rawFindings.push(...raw);
      findings.push(...scanner.withoutAllowed(destination, raw));
    }
  };
  for (const file of files) add(file, git(["show", `${base}:${file}`]));
  for (const directory of scanner.CURATED_VENDOR_ROOTS) {
    const parts = directory.split("/");
    const module = parts.slice(0, 2).join("/");
    const inside = parts.slice(2).join("/");
    const entry = git(["ls-tree", base, "--", module]);
    const commit = /^160000 commit ([a-f0-9]{40})\t/.exec(entry)?.[1];
    if (!commit) throw new Error(`Missing canonical gitlink for ${module}`);
    const cwd = `${root}/${module}`;
    const names = git(
      ["ls-tree", "-r", "-z", "--name-only", commit, "--", inside],
      cwd,
    )
      .split("\0")
      .filter((p) => p.endsWith(".rs"));
    for (const file of names)
      add(`${module}/${file}`, git(["show", `${commit}:${file}`], cwd));
  }
  const json = (path, optional = false) => {
    if (!git(["ls-tree", "--name-only", base, "--", path]).trim()) {
      if (optional) return null;
      throw new Error(`Canonical baseline missing: ${path}`);
    }
    const value = JSON.parse(git(["show", `${base}:${path}`]));
    if (!Array.isArray(value))
      throw new Error(`Canonical manifest must be an array: ${path}`);
    return value.map(relocated);
  };
  return {
    base,
    findings,
    rawFindings,
    changedPaths,
    baseline: json(BASELINE),
    extension: json(EXTENSION, true),
    allowances: json("scripts/ci/storage-bypass-allow-reasons.json", true),
  };
}

/** Initial audited exclusions must exist in canonical source; later exclusions cannot grow. */
export function validateAllowances(raw, records, history) {
  const errors = [];
  const current = new Set(raw.map(key));
  const oldSites = history ? new Set(history.rawFindings.map(key)) : null;
  const oldRecords =
    history?.allowances === null
      ? null
      : history
        ? new Set(
            history.allowances.map((x) =>
              key({ ...x, text: x.site, occurrence: 1 }),
            ),
          )
        : null;
  const seen = new Set();
  for (const record of records) {
    const id = key({ ...record, text: record.site, occurrence: 1 });
    if (seen.has(id))
      errors.push(`Duplicate audited allowance: ${record.path}`);
    seen.add(id);
    if (!current.has(id))
      errors.push(`Stale audited allowance: ${record.path}: ${record.site}`);
    if (oldRecords && !oldRecords.has(id))
      errors.push(`Audited allowance growth forbidden: ${record.path}`);
    if (history && !oldRecords && !oldSites.has(id))
      errors.push(
        `Initial allowance cannot approve a branch-added writer: ${record.path}`,
      );
  }
  return errors;
}
