#!/usr/bin/env python3
"""Simulate the dependency-floor effect of cutting crates, without editing Cargo.toml.

Why this exists
---------------
Additive per-dependency arithmetic is systematically wrong. Asking "what does
`cargo tree -i X` show?" for each candidate and summing the answers over-counts
shared subtrees and under-counts crates that only become droppable once a
*sibling* is also cut. The original audit's ~167 projection was produced that
way and was refuted; the real post-cut number is materially different.

This walks the actual resolved graph instead: cut a set of crates, redo the
reachability walk, count what survives. Cutting a cohort tells you the truth
about the cohort.

Why it parses `cargo tree` rather than `cargo metadata`
-------------------------------------------------------
`cargo metadata`'s resolve graph is *maximal*: `resolve.nodes[].deps` lists
dependencies that the active feature set never activates, and it keeps
target-specific edges that `--filter-platform` does not prune from the node
lists. A walk over it over-reports badly — measured here at 454 names against
cargo's actual 418, wrongly including dev-dependencies (`proptest`,
`rusty-fork`), Windows/macOS TLS edges (`openssl-sys`, `native-tls`,
`hyper-tls`), and unenabled reqwest features (`quinn`, `async-compression`).

Reproducing cargo's feature resolution correctly means resolving `dep:` syntax,
weak dependencies, and feature unification per node — a reimplementation whose
bugs would be invisible. `cargo tree` has already done that work exactly, so we
use its output as the graph source. The simulator is therefore calibrated by
construction, and `--cut-nothing` proves it against `kernel-floor.sh`.

Counting convention: the root crate (`openhuman`) is included, matching
`cargo tree` and `kernel-floor.sh`, so all three agree digit-for-digit.

Usage
-----
    scripts/dep-sim.py --cut-nothing
    scripts/dep-sim.py --cut arboard,enigo,rdev
    scripts/dep-sim.py --cut-file cohorts/web3.txt --json
    scripts/dep-sim.py --features "flows,voice" --cut git2
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

# Crates whose build script shells out to a C/C++/asm toolchain. Kept in sync
# with the same list in kernel-floor.sh — if you add one here, add it there.
NATIVE = {
    "libsqlite3-sys", "libgit2-sys", "libz-sys", "lzma-sys", "aws-lc-sys",
    "ring", "whisper-rs-sys", "openssl-sys", "zstd-sys", "bzip2-sys",
    "curl-sys", "onig_sys", "tree-sitter",
}

REPO = Path(__file__).resolve().parent.parent

# `cargo tree --prefix depth` emits e.g. "2serde v1.0.228" or
# "3foo v0.1.0 (*)" / "1bar v2.0 (proc-macro)".
LINE = re.compile(r"^(\d+)([^\s]+) v([^\s]+)")


def run_tree(features: str, default_features: bool, all_features: bool) -> str:
    cmd = ["cargo", "tree", "-e", "normal", "--prefix", "depth"]
    if all_features:
        cmd.append("--all-features")
    else:
        if not default_features:
            cmd.append("--no-default-features")
        if features:
            cmd += ["--features", features]
    env = {**os.environ, "GGML_NATIVE": "OFF"}
    out = subprocess.run(cmd, cwd=REPO, capture_output=True, text=True, env=env)
    if out.returncode != 0 or not out.stdout.strip():
        sys.exit(f"dep-sim: cargo tree failed:\n{out.stderr[-2000:]}")
    return out.stdout


def build_graph(text: str):
    """Parse `--prefix depth` output into (root, {node: {children}}).

    A line at depth d is a child of the most recent line at depth d-1. Nodes
    marked `(*)` are back-references whose subtree was expanded elsewhere; the
    *edge* they carry is still real, and because children are accumulated per
    node across the whole tree, the full expansion found elsewhere fills in the
    subtree. Nothing is lost by not recursing into them.
    """
    children: dict[tuple[str, str], set[tuple[str, str]]] = {}
    stack: dict[int, tuple[str, str]] = {}
    root: tuple[str, str] | None = None

    for raw in text.splitlines():
        m = LINE.match(raw.strip())
        if not m:
            continue
        depth = int(m.group(1))
        node = (m.group(2), m.group(3))
        stack[depth] = node
        children.setdefault(node, set())
        if depth == 0:
            root = root or node
        else:
            parent = stack.get(depth - 1)
            if parent is not None:
                children[parent].add(node)
    if root is None:
        sys.exit("dep-sim: could not find the root crate in cargo tree output")
    return root, children


def reachable(root, children, cut: set[str], global_cut: bool = False) -> set[tuple[str, str]]:
    """Reachability from root with `cut` crates removed.

    Two very different questions, and confusing them inflates every projection:

    `global_cut=False` (**the default — models a Cargo feature gate**). Removes
    only the ROOT's own edges to the cut crates. A crate with another parent
    survives, because gating openhuman's dependency on it cannot remove someone
    else's. This is what "make dep X optional behind a feature" actually does.

    `global_cut=True` (`--global-cut`) removes the crate from the graph however
    it is reached. That answers "what if this crate did not exist at all", which
    is only achievable by also changing the OTHER parents — usually an upstream
    PR, not a feature flag.

    The gap between them is not academic. The `encryption` cohort measures -17
    globally but **-4** as a gate, because `hmac`/`hkdf`/`x25519-dalek` arrive via
    `tinyplace` and `zeroize` via `aws-lc-rs -> rustls -> reqwest`. Quoting the
    global number as a gate's value overstates it by a factor of four.
    """
    if root[0] in cut:
        return set()
    seen = {root}
    stack = [root]
    while stack:
        cur = stack.pop()
        at_root = cur == root
        for child in children.get(cur, ()):
            # A gate can only sever the root's own edge; deeper edges belong to
            # other crates and survive it.
            if child[0] in cut and (global_cut or at_root):
                continue
            if child in seen:
                continue
            seen.add(child)
            stack.append(child)
    return seen


def summarize(nodes: set[tuple[str, str]]) -> dict:
    names = {n for n, _ in nodes}
    return {"packages": len(nodes), "names": len(names),
            "native": sorted(names & NATIVE)}


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--features", default="flows",
                    help="comma-separated feature list (default: flows)")
    ap.add_argument("--default-features", action="store_true",
                    help="keep default features on (default: off)")
    ap.add_argument("--all-features", action="store_true")
    ap.add_argument("--cut", default="",
                    help="comma-separated crate NAMES to remove from the graph")
    ap.add_argument("--cut-file", type=Path,
                    help="file with one crate name per line ('#' comments allowed)")
    ap.add_argument("--cut-nothing", action="store_true",
                    help="calibration mode: cut nothing, print the baseline")
    ap.add_argument("--global-cut", action="store_true",
                    help="remove the crate however it is reached, not just the root's "
                         "edge. Answers 'what if this crate did not exist', which a "
                         "feature gate CANNOT deliver when another parent pulls it in. "
                         "Default (off) models a real gate.")
    ap.add_argument("--expect-names", type=int,
                    help="exit non-zero unless the resulting name count matches")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    cut: set[str] = set()
    if not args.cut_nothing:
        cut |= {c.strip() for c in args.cut.split(",") if c.strip()}
        if args.cut_file:
            for line in args.cut_file.read_text().splitlines():
                line = line.split("#", 1)[0].strip()
                if line:
                    cut.add(line)

    root, children = build_graph(
        run_tree(args.features, args.default_features, args.all_features))

    base = summarize(reachable(root, children, set()))
    after = summarize(reachable(root, children, cut, args.global_cut)) if cut else base

    profile = ("all-features" if args.all_features
               else ("default + " if args.default_features else "no-default + ")
               + args.features)

    if args.json:
        print(json.dumps({
            "profile": profile, "cut": sorted(cut),
            "mode": "global" if args.global_cut else "gate",
            "baseline": base, "after": after,
            "delta_names": base["names"] - after["names"],
            "delta_packages": base["packages"] - after["packages"],
        }, indent=2))
    else:
        print(f"profile:  {profile}")
        print(f"baseline: {base['packages']} packages / {base['names']} names / "
              f"{len(base['native'])} native")
        if not cut:
            if base["native"]:
                print(f"          native: {' '.join(base['native'])}")
        else:
            mode = "global (not achievable by a feature gate)" if args.global_cut else "gate (root edge only)"
            print(f"mode:     {mode}")
            print(f"cut:      {', '.join(sorted(cut))}")
            print(f"after:    {after['packages']} packages / {after['names']} names / "
                  f"{len(after['native'])} native")
            print(f"delta:    -{base['packages'] - after['packages']} packages / "
                  f"-{base['names'] - after['names']} names")
            gone = set(base["native"]) - set(after["native"])
            if gone:
                print(f"          native removed:   {' '.join(sorted(gone))}")
            if after["native"]:
                print(f"          native remaining: {' '.join(after['native'])}")

    if args.expect_names is not None and after["names"] != args.expect_names:
        print(f"dep-sim: FAIL — expected {args.expect_names} names, got "
              f"{after['names']}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
