#!/usr/bin/env python3
"""Collect registry dependency attribution; never read mailbox data or credentials."""
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
metadata = json.loads(subprocess.check_output(
    ["bash", "scripts/cargo.sh", "metadata", "--locked", "--format-version", "1"], cwd=ROOT))
packages = sorted((p for p in metadata["packages"] if p["source"] or Path(p["manifest_path"]).is_relative_to(ROOT / "third_party")), key=lambda p: (p["name"], p["version"]))
parts = ["# Third-party notices\n",
         "Lightmail is distributed under GPL-3.0-or-later. Dependencies retain their own copyright and licenses.\n",
         "This inventory includes runtime, build, development and platform-specific packages resolved by Cargo.lock; not every package is linked into each build. License expressions come from package metadata, followed by available package license/notice files.\n",
         "Exact dependency sources are available from the linked crates.io versions and the vendored source archive attached to the release. UniFFI generates the FFI bindings. SQLite is included through rusqlite/libsqlite3-sys and is in the public domain. Apple SDKs and system frameworks are supplied by macOS, not redistributed here.\n",
         "Generated with `python3 scripts/third-party-notices.py`.\n"]
license_texts = {}
for package in packages:
    name, version = package["name"], package["version"]
    base = Path(package["manifest_path"]).parent
    parts += [f"\n## {name} {version}\n", f"License: `{package.get('license') or 'See package license file'}`\n\nSource: https://crates.io/crates/{name}/{version}\n"]
    if not package["source"]:
        parts.append(f"Local compatibility patch: [{base.relative_to(ROOT)}/PATCHES.md]({base.relative_to(ROOT)}/PATCHES.md).\n")
    files = [p for p in base.iterdir() if p.is_file() and p.name.upper().startswith(("LICENSE", "COPYING", "NOTICE", "AUTHORS", "UNLICENSE"))]
    files += [p for dirname in ("LICENSES", "licenses") for p in (base / dirname).glob("*") if p.is_file()]
    if package.get("license_file"):
        files.append(base / package["license_file"])
    for path in sorted(set(files)):
        if not path.is_file():
            continue
        try:
            content = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        content = "\n".join(line.rstrip() for line in content.splitlines()).rstrip()
        if content not in license_texts:
            license_texts[content] = len(license_texts) + 1
        reference = license_texts[content]
        parts.append(f"- [{path.relative_to(base)}](#license-text-{reference:04d})\n")
parts.append("\n# License texts\n\nIdentical license files are included once below; package entries above identify every original file.\n")
for content, reference in license_texts.items():
    fence = "`" * max(4, max((len(x) for x in __import__('re').findall(r'`+', content)), default=0) + 1)
    parts.append(f"\n## License text {reference:04d}\n\n<details><summary>Full license / notice</summary>\n\n{fence}text\n{content}\n{fence}\n\n</details>\n")
(ROOT / "THIRD_PARTY_NOTICES.md").write_text("\n".join(parts), encoding="utf-8")
print(f"Wrote notices for {len(packages)} locked dependency packages")
