#!/usr/bin/env python3
"""Fail when a Rust source file grows past the size limits.

Warn above WARN_LINES, fail above FAIL_LINES. Files in EXCEPTIONS may be larger,
but never beyond their recorded cap (so they cannot silently keep growing).
"""
import pathlib
import sys

WARN_LINES = 800
FAIL_LINES = 1500
EXCEPTIONS = {
    # One constructor that builds every GPU resource; dissolved by the render graph (roadmap 1.2.3).
    "crates/redox_render/src/renderer/init.rs": 1000,
}

root = pathlib.Path(__file__).resolve().parent.parent
failed = False
for path in sorted(root.glob("crates/**/*.rs")):
    if "target" in path.parts:
        continue
    rel = path.relative_to(root).as_posix()
    lines = sum(1 for _ in path.open(encoding="utf-8", errors="replace"))
    cap = EXCEPTIONS.get(rel)
    if cap is not None:
        if lines > cap:
            print(f"ERROR {rel}: {lines} lines exceeds its exception cap {cap}")
            failed = True
        continue
    if lines > FAIL_LINES:
        print(f"ERROR {rel}: {lines} lines (limit {FAIL_LINES})")
        failed = True
    elif lines > WARN_LINES:
        print(f"WARN  {rel}: {lines} lines (soft limit {WARN_LINES})")
sys.exit(1 if failed else 0)
