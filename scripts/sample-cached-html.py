#!/usr/bin/env python3
"""Copy only cached HTML into a local ignored probe directory, using read-only SQLite.

Explicitly opt-in: this script is for a user-authorized local real-mail review.
It never reads credentials, subjects, account records or settings, and never opens
MailEngine (which performs startup recovery). No network access is performed.
"""
import argparse
import collections
import hashlib
import json
from html.parser import HTMLParser
from pathlib import Path
import sqlite3

ROOT = Path(__file__).resolve().parent.parent


class Structure(HTMLParser):
    def __init__(self):
        super().__init__()
        self.tags = collections.Counter()
        self.inline_properties = collections.Counter()
        self.stylesheet_bytes = 0
        self.in_style = False
        self.table_depth = self.max_table_depth = 0

    def handle_starttag(self, tag, attrs):
        self.tags[tag] += 1
        self.in_style = self.in_style or tag == "style"
        if tag == "table":
            self.table_depth += 1
            self.max_table_depth = max(self.max_table_depth, self.table_depth)
        for key, value in attrs:
            if key == "style":
                for declaration in (value or "").split(";"):
                    name, sep, _ = declaration.partition(":")
                    if sep:
                        self.inline_properties[name.strip().lower()] += 1

    def handle_endtag(self, tag):
        if tag == "table":
            self.table_depth = max(0, self.table_depth - 1)
        if tag == "style":
            self.in_style = False

    def handle_data(self, data):
        if self.in_style:
            self.stylesheet_bytes += len(data.encode("utf-8"))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("database", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / "build") or "private" not in output.name:
        parser.error("Use a build/...private... directory; real mail must stay ignored")
    if output.exists() and any(output.iterdir()):
        parser.error("Choose a new empty directory to avoid mixing snapshots")
    output.mkdir(parents=True, exist_ok=True)
    connection = sqlite3.connect(args.database.resolve().as_uri() + "?mode=ro", uri=True)
    connection.execute("PRAGMA query_only=ON")
    rows = connection.execute("SELECT data FROM bodies ORDER BY touched DESC").fetchall()
    connection.close()
    seen = set()
    samples = []
    tags = collections.Counter()
    properties = collections.Counter()
    for (data,) in rows:
        html = json.loads(data).get("html", "")
        if not html.strip():
            continue
        digest = hashlib.sha256(html.encode("utf-8")).digest()
        if digest in seen:
            continue
        seen.add(digest)
        name = f"sample-{len(samples) + 1:02d}"
        (output / f"{name}.html").write_text(html, encoding="utf-8")
        structure = Structure()
        structure.feed(html)
        sample = {
            "sample": name,
            "htmlBytes": len(html.encode("utf-8")),
            "tableCount": structure.tags["table"],
            "maxTableDepth": structure.max_table_depth,
            "styleBytes": structure.stylesheet_bytes,
            "imageCount": structure.tags["img"],
        }
        samples.append(sample)
        tags.update(structure.tags)
        properties.update(structure.inline_properties)
    report = {"cachedBodies": len(rows), "uniqueHtmlBodies": len(samples),
              "samples": samples, "tags": dict(tags), "inlineProperties": dict(properties),
              "source": "read-only snapshot of already-cached HTML; no credentials or network"}
    (output / "manifest.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps({"cachedBodies": len(rows), "uniqueHtmlBodies": len(samples),
                      "maxTableDepth": max((s["maxTableDepth"] for s in samples), default=0),
                      "maxHtmlBytes": max((s["htmlBytes"] for s in samples), default=0)}))


if __name__ == "__main__":
    main()
