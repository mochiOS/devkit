#!/usr/bin/env python3
"""Check the background API with a real Kome compiler (no mock ELF generation)."""
import argparse
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--compiler", default="komec", help="Path to the native Kome compiler")
args = parser.parse_args()
package = Path(__file__).resolve().parents[1]
command = [args.compiler, "check", str(package / "tests/kome/background.kome")]
# Isolate the new contract from unrelated ViewKit wrapper compatibility tests.
for source in ["result.kome", "background.kome"]:
    command += ["--package-source", "appcore", str(package / "lib/src" / source)]
subprocess.run(command, check=True)
