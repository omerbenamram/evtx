#!/usr/bin/env python3
"""
Merge Rust + Zig UTF-16 escape benchmark CSVs into a markdown matrix.
"""

from __future__ import annotations

import argparse
import csv
from collections import defaultdict


def load_csv(path: str) -> dict[tuple[str, int], dict[str, float]]:
    out: dict[tuple[str, int], dict[str, float]] = {}
    with open(path, "r", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        for row in reader:
            pattern = row["pattern"]
            length = int(row["length"])
            out[(pattern, length)] = {
                "ns_per_iter": float(row["ns_per_iter"]),
                "ns_per_unit": float(row["ns_per_unit"]),
            }
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--rust", required=True)
    ap.add_argument("--zig", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    rust = load_csv(args.rust)
    zig = load_csv(args.zig)

    patterns = sorted({p for (p, _l) in rust.keys() | zig.keys()})
    lengths_by_pattern: dict[str, list[int]] = defaultdict(list)
    for (p, l) in rust.keys() | zig.keys():
        lengths_by_pattern[p].append(l)
    for p in patterns:
        lengths_by_pattern[p] = sorted(set(lengths_by_pattern[p]))

    with open(args.out, "w", encoding="utf-8") as f:
        f.write("| pattern | length | rust ns/unit | zig ns/unit | ratio |\n")
        f.write("| --- | ---: | ---: | ---: | ---: |\n")
        for p in patterns:
            for l in lengths_by_pattern[p]:
                r = rust.get((p, l))
                z = zig.get((p, l))
                if not r or not z:
                    continue
                ratio = r["ns_per_unit"] / z["ns_per_unit"] if z["ns_per_unit"] else 0.0
                f.write(
                    f"| {p} | {l} | {r['ns_per_unit']:.6f} | {z['ns_per_unit']:.6f} | {ratio:.2f} |\n"
                )

    print(f"Wrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
