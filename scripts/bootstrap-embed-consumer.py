#!/usr/bin/env python3
"""Create a pinned standalone consumer without hand-maintaining Cargo patches."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
from urllib.parse import urlsplit

try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10 in the Ubuntu 22.04 CI image.
    import tomli as tomllib

REPOSITORY = "https://github.com/tinyhumansai/openhuman.git"


def git(*args: str) -> str:
    """Run Git without echoing a source URL, credential or helper's output."""
    result = subprocess.run(["git", *args], capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError("Git could not prepare the pinned source checkout")
    return result.stdout.strip()


def consumer_manifest(checkout: Path, unused: frozenset[str] = frozenset()) -> str:
    """Read patches from this pin, checking they stay in the source bundle."""
    workspace = tomllib.loads((checkout / "Cargo.toml").read_text())
    lines = [
        "# Generated from the pinned OpenHuman workspace; do not mirror patches by hand.",
        "[workspace]",
        'exclude = ["vendor"]',
        "[package]",
        'name = "embed-consumer"',
        'version = "0.0.0"',
        'edition = "2024"',
        'rust-version = "1.99.1"',
        "publish = false",
        "[features]",
        "default = []",
        'embed = ["dep:openhuman-embed"]',
        "[dependencies]",
        'openhuman-embed = { path = "vendor/openhuman/crates/openhuman-embed", optional = true, default-features = false }',
    ]
    for registry, packages in workspace.get("patch", {}).items():
        lines.append(f"[patch.{json.dumps(registry)}]")
        for name, spec in packages.items():
            if name in unused:
                continue
            # A new patch form is a packaging-contract change. Refuse it
            # rather than quietly generating a different resolution graph.
            if (not isinstance(spec, dict) or set(spec) != {"path"}
                    or not isinstance(spec["path"], str)):
                raise ValueError(f"Unsupported workspace patch form: {name}")
            package = (checkout / spec["path"]).resolve()
            if not package.is_relative_to(checkout.resolve()):
                raise ValueError(f"Workspace patch leaves the source checkout: {name}")
            if not (package / "Cargo.toml").is_file():
                raise ValueError(f"Workspace patch is not initialized: {name}")
            relative = package.relative_to(checkout.resolve()).as_posix()
            lines.append(f"{json.dumps(name)} = {{ path = {json.dumps('vendor/openhuman/' + relative)} }}")
    lines += ["[profile.dev.package.\"*\"]", "debug = false"]
    return "\n".join(lines) + "\n"


def bootstrap(source: str, rev: str, destination: Path, offline: bool = False) -> None:
    """Clone only committed source, verify its pin, then generate a consumer."""
    if not re.fullmatch(r"[0-9a-fA-F]{40}", rev):
        raise ValueError("--rev must be a full 40-character Git commit SHA")
    parsed = urlsplit(source)
    if parsed.query or parsed.fragment or parsed.password or (
        parsed.scheme in {"http", "https"} and parsed.username
    ):
        raise ValueError("A credential-bearing source URL is not supported; use a Git credential helper")
    destination = destination.resolve()
    # mkdir is exclusive: an existing host checkout is never overwritten.
    destination.mkdir(parents=True, exist_ok=False)
    try:
        checkout = destination / "vendor" / "openhuman"
        checkout.parent.mkdir()
        git("clone", "--quiet", "--no-checkout", "--", source, str(checkout))
        git("-C", str(checkout), "checkout", "--quiet", "--detach", rev)
        if git("-C", str(checkout), "rev-parse", "HEAD").lower() != rev.lower():
            raise RuntimeError("The source checkout does not match the requested pin")
        git("-C", str(checkout), "submodule", "update", "--init", "--recursive")
        modules = git("-C", str(checkout), "submodule", "status", "--recursive")
        if any(line.startswith(("-", "+", "U")) for line in modules.splitlines()):
            raise RuntimeError("A submodule does not match its recorded pin")
        (destination / "Cargo.toml").write_text(consumer_manifest(checkout))
        (destination / "src").mkdir()
        (destination / "src" / "lib.rs").write_text(
            '//! Host entry point. Enable `embed` only for workloads using OpenHuman.\n'
            '#[cfg(feature = "embed")]\npub use openhuman_embed as embed;\n'
        )
        (destination / "rust-toolchain.toml").write_text(
            '[toolchain]\nchannel = "1.99.1"\nprofile = "minimal"\n'
        )
        lock = checkout / "Cargo.lock"
        if lock.is_file():
            shutil.copyfile(lock, destination / "Cargo.lock")
        resolve_lock(destination, offline)
        resolved = tomllib.loads((destination / "Cargo.lock").read_text())
        unused = frozenset(entry["name"] for entry in resolved.get("patch", {}).get("unused", []))
        if unused:
            # Cargo orders unused patches nondeterministically, which can
            # make --locked refuse an unchanged graph. Remove only entries
            # the resolver proved unused; keep every active pinned patch.
            (destination / "Cargo.toml").write_text(consumer_manifest(checkout, unused))
            resolve_lock(destination, offline)
        strip_git_metadata(checkout)
        (destination / "OPENHUMAN_REV").write_text(rev.lower() + "\n")
        (destination / ".gitignore").write_text("/target/\n")
    except Exception:
        # Only this invocation's newly created destination is removed.
        shutil.rmtree(destination)
        raise


def strip_git_metadata(checkout: Path) -> None:
    """Turn the verified clone into source files a host can commit normally."""
    root_metadata = checkout / ".git"
    if root_metadata.is_dir():
        shutil.rmtree(root_metadata)
    elif root_metadata.exists():
        root_metadata.unlink()
    for metadata in list(checkout.rglob(".git")):
        if metadata.is_dir():
            shutil.rmtree(metadata)
        else:
            metadata.unlink()


def resolve_lock(destination: Path, offline: bool) -> None:
    """Resolve the host lockfile, keeping provider/helper output private."""
    command = ["cargo", "generate-lockfile", "--manifest-path", str(destination / "Cargo.toml")]
    if offline:
        command.append("--offline")
    result = subprocess.run(command, cwd=destination, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError("Cargo could not resolve the consumer lockfile; check registry/cache access")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", default=REPOSITORY, help="Git repository URL or local checkout")
    parser.add_argument("--rev", required=True, help="Full OpenHuman commit SHA")
    parser.add_argument("--destination", type=Path, required=True, help="New consumer directory")
    parser.add_argument("--offline", action="store_true", help="Resolve Cargo using cached registry sources only")
    args = parser.parse_args()
    try:
        bootstrap(args.source, args.rev, args.destination, args.offline)
    except (OSError, ValueError, RuntimeError) as error:
        parser.exit(1, f"Bootstrap failed: {error}\n")
    print(f"Consumer ready at {args.destination}; OpenHuman pin {args.rev.lower()}")
    print("Default build leaves Embed disabled; enable it with cargo check --features embed")


if __name__ == "__main__":
    main()
