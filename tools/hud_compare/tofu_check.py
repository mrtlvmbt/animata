#!/usr/bin/env python3
"""Static tofu guard: find characters in HUD string literals that no loaded font can render.

A glyph the font lacks renders as a tofu box (□) — e.g. U+2192 "→" in the vendored IBM Plex subset.
This scans the Rust UI sources for string/char literals and flags any non-ASCII codepoint that is
NOT in the bundled IBM Plex Sans/Mono cmaps and NOT in the Phosphor private-use range (egui_phosphor
glyphs, which the app also registers). Pure static — runs without the app, exits non-zero on a hit so
it can gate CI.

    python3 tools/hud_compare/tofu_check.py
"""
import re, sys
from pathlib import Path
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parents[2]
FONTS = [
    ROOT / "assets/fonts/IBMPlexSans-Regular.ttf",
    ROOT / "assets/fonts/IBMPlexMono-Regular.ttf",
]
SRC = [ROOT / "crates/animata/src/ui", ROOT / "crates/animata/src/main.rs"]
PUA = range(0xE000, 0xF900)  # Phosphor icon glyphs live here (registered in both families)

STR_LIT = re.compile(r'"((?:\\.|[^"\\])*)"')       # double-quoted string literals
UESC = re.compile(r'\\u\{([0-9A-Fa-f]+)\}')         # \u{XXXX} escapes


def covered(cmap, cp):
    return cp < 128 or cp in cmap or cp in PUA


def chars_of(literal):
    """Yield codepoints of a Rust string literal, resolving \\u{...}; ignore other escapes."""
    i = 0
    # First pull out \u{...} escapes, then walk remaining literal chars (escapes like \n are ASCII).
    for m in UESC.finditer(literal):
        yield int(m.group(1), 16)
    stripped = UESC.sub("", literal)
    stripped = re.sub(r'\\.', "", stripped)  # drop other escape sequences (all ASCII)
    for ch in stripped:
        yield ord(ch)


def main():
    cmap = set()
    for f in FONTS:
        cmap |= set(TTFont(f).getBestCmap().keys())

    files = []
    for s in SRC:
        files += [s] if s.is_file() else sorted(s.rglob("*.rs"))

    hits = []
    for f in files:
        for ln, line in enumerate(f.read_text(encoding="utf-8").splitlines(), 1):
            for m in STR_LIT.finditer(line):
                for cp in chars_of(m.group(1)):
                    if not covered(cmap, cp):
                        hits.append((f.relative_to(ROOT), ln, cp, chr(cp)))

    if hits:
        print("TOFU: characters with no glyph in IBM Plex (and not Phosphor PUA):")
        for rel, ln, cp, ch in hits:
            print(f"  {rel}:{ln}  U+{cp:04X} '{ch}' — renders as a tofu box")
        print(f"\n{len(hits)} hit(s). Use a Phosphor glyph (e.g. ph::ARROW_RIGHT) or an ASCII form.")
        sys.exit(1)
    print("OK — every non-ASCII char in HUD string literals has a glyph.")


if __name__ == "__main__":
    main()
