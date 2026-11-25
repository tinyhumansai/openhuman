// Pure pin-status logic for scripts/modules/pin-status.mjs
// (tinyhumansai/openhuman#7615, epic #7523).
//
// check-module-pins.mjs is the strict CI gate and says only pass/fail. This is
// the local, advisory view of the same two pins: it tells a developer WHICH way
// a module's gitlink differs from the registry `version`, because "ahead" means
// they are running a locally compiled module that CI will refuse.

import { classifyPin } from "./module-pins.mjs";

/** `vMAJOR.MINOR.PATCH[-N-gHASH]` from `git describe --tags`, or null. */
export function parseDescribe(describe) {
  const m = String(describe ?? "")
    .trim()
    .match(/^(v(\d+)\.(\d+)\.(\d+))(?:-(\d+)-g([0-9a-f]+))?$/);
  if (!m) return null;
  return {
    tag: m[1],
    parts: [Number(m[2]), Number(m[3]), Number(m[4])],
    commitsPast: m[5] ? Number(m[5]) : 0,
  };
}

/** Negative, zero or positive as `a` is older than, equal to or newer than `b`. */
export function compareParts(a, b) {
  for (let i = 0; i < 3; i++) {
    if (a[i] !== b[i]) return a[i] - b[i];
  }
  return 0;
}

export const AHEAD_MESSAGE =
  "gitlink ahead of released pin — you're running a locally compiled module; " +
  "publish a release and re-pin before CI will pass";

/**
 * Classify one module.
 *
 * `actual` is `git describe --tags` of the submodule; `wantedIsAncestor` says
 * whether the registry's tag is an ancestor of the submodule HEAD (`null` when
 * unknown, for instance the tag is not fetched); `exemption` is the matching
 * module-pin-exemptions.json entry.
 *
 * Kinds: `in-sync`, `ahead`, `behind`, `diverged`, `exempt`.
 */
export function classifyStatus({
  id,
  version,
  submodulePath,
  actual,
  wantedIsAncestor = null,
  exemption,
  sourceParityContext,
}) {
  const strict = classifyPin({
    id,
    version,
    submodulePath,
    actual,
    exemption,
    sourceParityContext,
  });
  if (strict.kind === "exempt") {
    return {
      id,
      kind: "exempt",
      actual,
      message: strict.sourceParity
        ? `exempt (verified test-only source parity at ${actual}): ${strict.reason}`
        : `exempt (declared drift accepted): ${strict.reason}`,
    };
  }
  const note =
    strict.kind === "stale-exemption"
      ? " (its module-pin-exemptions.json entry is stale; delete it)"
      : strict.kind === "exemption-widened"
        ? " (the drift moved past its module-pin-exemptions.json entry)"
        : strict.kind === "invalid-source-parity"
          ? ` (${strict.message})`
          : "";
  if (strict.kind === "match" || strict.kind === "stale-exemption") {
    return { id, kind: "in-sync", actual, message: `in sync${note}` };
  }

  const wanted = parseDescribe(`v${version}`);
  const got = parseDescribe(actual);
  const diverged = {
    id,
    kind: "diverged",
    actual,
    message:
      `behind/diverged: registry pins v${version} but ${submodulePath} is at ` +
      `${actual}, which does not descend from it${note}`,
  };
  if (!wanted || !got || wantedIsAncestor === false) return diverged;

  const order = compareParts(got.parts, wanted.parts);
  if (order > 0 || (order === 0 && got.commitsPast > 0)) {
    return { id, kind: "ahead", actual, message: `${AHEAD_MESSAGE}${note}` };
  }
  return {
    id,
    kind: "behind",
    actual,
    message:
      `behind/diverged: registry pins v${version} but ${submodulePath} is at ` +
      `${actual}; check out the pinned release or re-pin${note}`,
  };
}

/** Ids whose gitlink is ahead of the registry pin, for the runtime warning. */
export function aheadIds(statuses) {
  return statuses.filter((s) => s.kind === "ahead").map((s) => s.id);
}

/** One printable line per module. */
export function formatStatus(status) {
  return `${status.id.padEnd(16)} ${status.kind.padEnd(9)} ${status.message}`;
}
