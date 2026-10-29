#!/usr/bin/env python3
"""Check the background API with a real Kome compiler (no mock ELF generation)."""
import argparse
from pathlib import Path
import subprocess
import tomllib

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--compiler", default="komec", help="Path to the native Kome compiler")
parser.add_argument("--viewkit", type=Path, help="ViewKit Kome package directory; checks all AppCore/ViewKit wrappers too")
args = parser.parse_args()
package = Path(__file__).resolve().parents[1]
command = [args.compiler, "check", str(package / "tests/kome/background.kome")]
if args.viewkit:
    for name, directory in [("appcore", package), ("viewkit", args.viewkit.resolve())]:
        manifest = tomllib.loads((directory / "Kome.toml").read_text())
        for source in [manifest["lib"]["source"], *manifest["lib"].get("sources", [])]:
            command += ["--package-source", name, str(directory / source)]
else:
    for source in ["result.kome", "background.kome"]:
        command += ["--package-source", "appcore", str(package / "lib/src" / source)]
subprocess.run(command, check=True)
