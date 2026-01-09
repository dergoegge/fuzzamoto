#!/usr/bin/env python3

import sys
import subprocess
from pathlib import Path

if len(sys.argv) != 2:
    print(f"usage: {sys.argv[0]} <BUG NAME>", file=sys.stderr)
    sys.exit(1)

name = sys.argv[1]
yaml_path = Path("bugs") / name / "benchmark.yaml"

if not yaml_path.is_file():
    print(f"benchmark.yaml not found: {yaml_path}", file=sys.stderr)
    sys.exit(1)

commit = None
with yaml_path.open() as f:
    for line in f:
        line = line.strip()
        if line.startswith(("commit:")):
            commit = line.split(":", 1)[1].strip()
            break

if not commit:
    print(f"{yaml_path} is malformed", file=sys.stderr)
    sys.exit(1)

cmd = [
    "docker", "build",
    "-f", "Dockerfile.bug",
    "-t", f"fuzzamoto-libafl-{name}",
    ".",
    "--build-arg", f"BITCOIN_COMMIT={commit}",
    "--build-arg", f"BUG_PATCH={name}",
]
subprocess.check_call(cmd)
print("Built ", f"fuzzamoto-libafl-{name}")
