#!/usr/bin/env python3
"""Bump muninn-coinbase-connector, record the changelog, commit, and push.

The bump defaults to minor. Pass --major or --patch for the other components.
The positional message is the git commit message, the changelog entry, and the
message on the vMAJOR.MINOR.PATCH tag. Pushing that tag starts the release build.
"""

from __future__ import annotations

import argparse
import datetime as dt
import re
import subprocess
import sys
from pathlib import Path

PACKAGE = "muninn-coinbase-connector"
VERSION_LINE = re.compile(r'^version\s*=\s*"(\d+)\.(\d+)\.(\d+)"\s*$')
SECRET_NAMES = {
    ".env",
    ".env.local",
    "credentials.json",
    "id_rsa",
    "id_ed25519",
}


def bump(version: str, kind: str) -> str:
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", version)
    if match is None:
        raise ValueError(f"version {version} is not MAJOR.MINOR.PATCH")
    major, minor, patch = (int(part) for part in match.groups())
    if kind == "major":
        return f"{major + 1}.0.0"
    if kind == "minor":
        return f"{major}.{minor + 1}.0"
    if kind == "patch":
        return f"{major}.{minor}.{patch + 1}"
    raise ValueError(kind)


def package_version(text: str) -> str:
    in_package = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped == "[package]":
            in_package = True
            continue
        if in_package and stripped.startswith("["):
            break
        if in_package:
            match = VERSION_LINE.match(stripped)
            if match:
                return ".".join(match.groups())
    raise ValueError("Cargo.toml has no [package] version")


def replace_package_version(text: str, new: str) -> str:
    lines = text.splitlines(keepends=True)
    in_package = False
    for index, line in enumerate(lines):
        stripped = line.strip()
        if stripped == "[package]":
            in_package = True
            continue
        if in_package and stripped.startswith("["):
            break
        if in_package and VERSION_LINE.match(stripped):
            newline = "\n" if line.endswith("\n") else ""
            lines[index] = f'version = "{new}"{newline}'
            return "".join(lines)
    raise ValueError("Cargo.toml has no [package] version")


def replace_lock_version(text: str, name: str, new: str) -> str:
    lines = text.splitlines(keepends=True)
    needle = f'name = "{name}"'
    for index, line in enumerate(lines):
        if line.strip() != needle:
            continue
        if index + 1 < len(lines) and lines[index + 1].lstrip().startswith("version = "):
            newline = "\n" if lines[index + 1].endswith("\n") else ""
            indent = lines[index + 1][: len(lines[index + 1]) - len(lines[index + 1].lstrip())]
            lines[index + 1] = f'{indent}version = "{new}"{newline}'
            return "".join(lines)
    raise ValueError(f"Cargo.lock has no package {name}")


def changelog_entry(version: str, message: str, day: dt.date) -> str:
    body = message.strip()
    if not body:
        raise ValueError("commit message is empty")
    return f"## [{version}] - {day.isoformat()}\n\n{body}\n"


def prepend_changelog(text: str, entry: str) -> str:
    match = re.search(r"(?m)^## \[", text)
    block = entry if entry.endswith("\n") else entry + "\n"
    if match is None:
        base = text.rstrip() + "\n\n" if text.strip() else "# Changelog\n\n"
        return base + block
    return text[: match.start()] + block + "\n" + text[match.start() :]


def looks_secret(path: str) -> bool:
    name = Path(path).name
    if name in SECRET_NAMES or name.startswith(".env"):
        return True
    return Path(path).suffix in {".pem", ".key"}


def run(cmd: list[str], cwd: Path, input_text: str | None = None) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        cmd,
        cwd=cwd,
        input=input_text,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        sys.exit(result.returncode)
    return result


def git_lines(cmd: list[str], cwd: Path) -> list[str]:
    result = subprocess.run(cmd, cwd=cwd, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        sys.exit(result.returncode)
    return [line for line in result.stdout.splitlines() if line]


def release(root: Path, kind: str, message: str, today: dt.date | None = None) -> str:
    cargo = root / "Cargo.toml"
    lock = root / "Cargo.lock"
    changelog = root / "CHANGELOG.md"
    current = package_version(cargo.read_text())
    new = bump(current, kind)
    cargo.write_text(replace_package_version(cargo.read_text(), new))
    if lock.exists():
        lock.write_text(replace_lock_version(lock.read_text(), PACKAGE, new))
    day = today or dt.date.today()
    entry = changelog_entry(new, message, day)
    existing = changelog.read_text() if changelog.exists() else "# Changelog\n\n"
    changelog.write_text(prepend_changelog(existing, entry))
    return new


def pending_paths(root: Path) -> list[str]:
    result = subprocess.run(
        ["git", "status", "--porcelain"],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        sys.exit(result.returncode)
    paths = []
    for line in result.stdout.splitlines():
        entry = line[3:]
        if " -> " in entry:
            entry = entry.split(" -> ", 1)[1]
        paths.append(entry.strip().strip('"'))
    return paths


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--major", action="store_true", help="bump the major version")
    group.add_argument("--patch", action="store_true", help="bump the patch version")
    parser.add_argument("message", help="commit message and changelog entry")
    args = parser.parse_args()

    root = Path(
        subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            text=True,
            capture_output=True,
            check=True,
        ).stdout.strip()
    )
    secrets = [path for path in pending_paths(root) if looks_secret(path)]
    if secrets:
        sys.exit(f"refusing to commit {', '.join(secrets)}")
    kind = "major" if args.major else "patch" if args.patch else "minor"
    try:
        new = release(root, kind, args.message)
    except ValueError as err:
        sys.exit(str(err))
    run(["git", "add", "-A"], root)
    staged = git_lines(["git", "diff", "--cached", "--name-only"], root)
    if not staged:
        sys.exit("nothing to commit")
    message = args.message.strip() + "\n"
    run(["git", "commit", "-F", "-"], root, input_text=message)
    tag = f"v{new}"
    run(["git", "tag", "-a", tag, "-m", message], root)
    upstream = subprocess.run(
        ["git", "rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )
    if upstream.returncode == 0:
        run(["git", "push", "--follow-tags"], root)
    else:
        run(["git", "push", "-u", "origin", "HEAD", "--follow-tags"], root)
    print(tag)


if __name__ == "__main__":
    main()
