#!/usr/bin/env python3
"""Preserve registry/Git name-version collisions without changing dependency code.

Only the source archive is patched; the release checkout/lock stays untouched.
Cargo vendor cannot merge identical name/version pairs from distinct sources.
"""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib

root = Path(__file__).resolve().parent.parent
stage = Path(sys.argv[1]).resolve()
cargo = ["bash", str(root / "scripts/cargo.sh")]
lock = tomllib.loads((root / "Cargo.lock").read_text())["package"]
counts = {}
for package in lock:
    if package.get("source"):
        key = (package["name"], package["version"])
        counts[key] = counts.get(key, 0) + 1
collisions = [p for p in lock if counts.get((p["name"], p["version"]), 0) > 1 and p.get("source", "").startswith("registry+")]
if not collisions:
    sys.exit(0)
metadata = json.loads(subprocess.check_output(cargo + ["metadata", "--locked", "--format-version", "1"], cwd=root))
patches = []
records = []
for package in collisions:
    assert package["source"] == "registry+https://github.com/rust-lang/crates.io-index", "Review non-crates.io registry before packaging"
    match = next(p for p in metadata["packages"] if all(p[k] == package[k] for k in ("name", "version", "source")))
    source = Path(match["manifest_path"]).parent
    relative = Path("vendor-path") / (package["name"] + "-" + package["version"])
    target = stage / relative
    shutil.copytree(source, target)
    # This is the exact registry package from Cargo.lock, including its license.
    checksums = json.loads((target / ".cargo-checksum.json").read_text())
    assert checksums["package"] == package["checksum"]
    patches.append(f'{package["name"]} = {{ path = "{relative.as_posix()}" }}')
    records.append({k: package[k] for k in ("name", "version", "source", "checksum")})
manifest = stage / "Cargo.toml"
text = manifest.read_text()
assert "[patch.crates-io]" in text
text = text.replace("[patch.crates-io]", "[patch.crates-io]\n" + "\n".join(patches), 1)
manifest.write_text(text)
subprocess.run(cargo + ["metadata", "--offline", "--manifest-path", str(manifest), "--format-version", "1"], cwd=root, stdout=subprocess.DEVNULL, check=True)
updated = tomllib.loads((stage / "Cargo.lock").read_text())["package"]
def identity(package):
    return tuple(package.get(k) for k in ("name", "version", "source", "checksum"))
expected = {identity(p) for p in lock} - {identity(p) for p in collisions}
expected |= {(p["name"], p["version"], None, None) for p in collisions}
assert {identity(p) for p in updated} == expected, "Archive preparation changed another dependency"
(stage / "SOURCE_PACKAGING.json").write_text(json.dumps({"registry_path_overrides": records, "reason": "Cargo vendor cannot merge same-name/same-version registry and Git packages. Registry source files are unchanged; only archive manifest/lock source locations differ from SOURCE_COMMIT.txt."}, indent=2) + "\n")
print(f"Preserved {len(collisions)} dependency source collision(s) in archive-local paths")
