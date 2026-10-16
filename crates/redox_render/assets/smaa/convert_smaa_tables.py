import re
import os
import sys
from pathlib import Path

ROOT = Path(r"c:\Users\yann1n\RustroverProjects\redox_engine")
OUT = ROOT / "crates" / "redox_render" / "assets" / "smaa"
TEMP = Path(os.environ.get("TEMP", r"C:\Users\yann1n\AppData\Local\Temp"))

def parse_c_byte_array(header_path: Path, array_name: str) -> bytes:
    text = header_path.read_text(encoding="utf-8", errors="replace")
    m = re.search(
        rf"static\s+const\s+unsigned\s+char\s+{re.escape(array_name)}\s*\[\s*\]\s*=\s*\{{(.*?)\}};",
        text,
        re.DOTALL,
    )
    if not m:
        raise SystemExit(f"Could not find {array_name}[] in {header_path}")
    body = m.group(1)
    return bytes(int(h, 16) for h in re.findall(r"0x([0-9A-Fa-f]{1,2})", body))

def expected_size(header_path: Path, size_macro: str) -> int | None:
    text = header_path.read_text(encoding="utf-8", errors="replace")
    m = re.search(rf"#define\s+{re.escape(size_macro)}\s+\([^)]+\)|#define\s+{re.escape(size_macro)}\s+(\d+)", text)
    if not m:
        return None
    if m.lastindex and m.group(1):
        return int(m.group(1))
    # compute from WIDTH/HEIGHT/PITCH macros
    defs = dict(re.findall(r"#define\s+(\w+)\s+(\d+)", text))
    prefix = size_macro.replace("_SIZE", "")
    if f"{prefix}_HEIGHT" in defs and f"{prefix}_PITCH" in defs:
        return int(defs[f"{prefix}_HEIGHT"]) * int(defs[f"{prefix}_PITCH"])
    return None

def write_bin(data: bytes, path: Path) -> int:
    path.write_bytes(data)
    return path.stat().st_size

def main() -> int:
    sources = [
        (TEMP / "AreaTex.h", "areaTexBytes", "AREATEX_SIZE", OUT / "AreaTexDX10.bin"),
        (TEMP / "SearchTex.h", "searchTexBytes", "SEARCHTEX_SIZE", OUT / "SearchTex.bin"),
    ]
    OUT.mkdir(parents=True, exist_ok=True)
    ok = True
    for header, array, size_macro, out_path in sources:
        if not header.is_file():
            print(f"Missing: {header}", file=sys.stderr)
            return 1
        data = parse_c_byte_array(header, array)
        exp = expected_size(header, size_macro)
        written = write_bin(data, out_path)
        print(f"{out_path.name}: {written} bytes", end="")
        if exp is not None:
            print(f" (header {size_macro}={exp})", end="")
            if written != exp or len(data) != exp:
                print(" MISMATCH", file=sys.stderr)
                ok = False
            else:
                print(" OK")
        else:
            print()
    return 0 if ok else 1

if __name__ == "__main__":
    raise SystemExit(main())
