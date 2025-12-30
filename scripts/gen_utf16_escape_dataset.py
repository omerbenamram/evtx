#!/usr/bin/env python3
"""
Generate a shared UTF-16LE escape benchmark dataset for Rust + Zig.

Binary format (little-endian):
  - magic: b"UTFB"
  - u32 version (1)
  - u32 case_count
  - repeated cases:
      - u32 pattern_len
      - pattern bytes (utf-8)
      - u32 length_units
      - u32 byte_len
      - utf16le bytes
"""

from __future__ import annotations

import argparse
import struct
from dataclasses import dataclass


@dataclass(frozen=True)
class Case:
    pattern: str
    length_units: int
    utf16le: bytes


def _repeat_units(units: list[int], length: int) -> list[int]:
    out: list[int] = []
    if not units:
        return out
    i = 0
    while len(out) < length:
        out.append(units[i % len(units)])
        i += 1
    return out


def _build_cases(lengths: list[int]) -> list[Case]:
    patterns: dict[str, list[int]] = {
        "ascii_plain": [0x0061],  # 'a'
        "ascii_esc": [
            0x0022,  # "
            0x005C,  # \
            0x000A,  # \n
            0x000D,  # \r
            0x0009,  # \t
            0x0008,  # \b
            0x000C,  # \f
            0x001F,  # unit separator
            0x0041,  # A
        ],
        "latin1": [0x00E9],  # é
        "bmp": [0x2603],  # ☃
        "surrogate": [0xD83D, 0xDE00],  # 😀
    }

    cases: list[Case] = []
    for pattern, units in patterns.items():
        for length in lengths:
            if length <= 0:
                continue
            if pattern == "surrogate":
                # Fill with full surrogate pairs; if odd, pad final unit with 'A'.
                pair_count = length // 2
                out_units: list[int] = []
                for _ in range(pair_count):
                    out_units.extend(units)
                if length % 2 == 1:
                    out_units.append(0x0041)
            else:
                out_units = _repeat_units(units, length)

            utf16le = b"".join(struct.pack("<H", u) for u in out_units)
            cases.append(Case(pattern=pattern, length_units=length, utf16le=utf16le))
    return cases


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True, help="Output dataset path")
    ap.add_argument(
        "--lengths",
        default="1,2,3,4,5,8,12,16,24,32,48,64,96,128",
        help="Comma-separated UTF-16 unit lengths",
    )
    args = ap.parse_args()

    lengths = [int(x) for x in args.lengths.split(",") if x.strip()]
    cases = _build_cases(lengths)

    with open(args.out, "wb") as f:
        f.write(b"UTFB")
        f.write(struct.pack("<I", 1))
        f.write(struct.pack("<I", len(cases)))
        for case in cases:
            pat = case.pattern.encode("utf-8")
            f.write(struct.pack("<I", len(pat)))
            f.write(pat)
            f.write(struct.pack("<I", case.length_units))
            f.write(struct.pack("<I", len(case.utf16le)))
            f.write(case.utf16le)

    print(f"Wrote {len(cases)} cases to {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
