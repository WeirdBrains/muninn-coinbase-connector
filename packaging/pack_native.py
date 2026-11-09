#!/usr/bin/env python3
"""Pack the release library, LICENSE, and README into dist/."""

from __future__ import annotations

import os
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LIB = {
    "linux": "libmuninn_coinbase_connector.so",
    "darwin": "libmuninn_coinbase_connector.dylib",
    "win32": "muninn_coinbase_connector.dll",
}
OS_NAME = {"linux": "linux", "darwin": "macos", "win32": "windows"}


def main() -> None:
    version = os.environ["VERSION"]
    arch = os.environ["ARCH"]
    target = os.environ.get("TARGET", "")
    lib_name = LIB[sys.platform]
    os_name = OS_NAME[sys.platform]
    release = ROOT / "target"
    if target:
        release = release / target
    lib = release / "release" / lib_name
    if not lib.is_file():
        sys.exit(f"missing {lib}")

    stage = ROOT / "dist" / "stage"
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    shutil.copy2(lib, stage / lib_name)
    shutil.copy2(ROOT / "LICENSE", stage / "LICENSE")
    shutil.copy2(ROOT / "README.md", stage / "README.md")

    base = ROOT / "dist" / f"muninn-coinbase-connector-{version}-{os_name}-{arch}"
    fmt = "zip" if sys.platform == "win32" else "gztar"
    archive = shutil.make_archive(str(base), fmt, root_dir=stage)
    shutil.rmtree(stage)
    print(archive)


if __name__ == "__main__":
    main()
