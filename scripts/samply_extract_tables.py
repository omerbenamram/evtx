#!/usr/bin/env python3
"""
Extract simple markdown tables from a samply (Firefox Profiler) JSON profile + syms sidecar.

Outputs (matching PERF.md conventions):
  - top_leaves_<label>_cpu.md
  - top_inclusive_<label>_cpu.md
  - leaf_callers_<label>.md

The "raw" profiles remain the authoritative artifacts:
  - <profile>.json.gz
  - <profile>.json.syms.json
"""

from __future__ import annotations

import argparse
import bisect
import gzip
import json
import os
from dataclasses import dataclass
from typing import Any


def _load_json(path: str) -> Any:
    with open(path, "rb") as f:
        data = f.read()
    if path.endswith(".gz"):
        data = gzip.decompress(data)
    return json.loads(data)


def _norm_hex(s: str) -> str:
    return s.upper()


@dataclass(frozen=True)
class Symbolicator:
    libs: list[dict[str, Any]]
    lib_entries: list[dict[str, Any] | None]
    preprocessed: dict[int, tuple[list[int], list[int], list[str]]]

    @classmethod
    def from_profile_and_syms(
        cls, profile: dict[str, Any], syms: dict[str, Any]
    ) -> "Symbolicator":
        string_table = syms.get("string_table") or []
        syms_data = syms.get("data") or []

        syms_by_code: dict[str, dict[str, Any]] = {}
        syms_by_name: dict[str, dict[str, Any]] = {}
        preprocessed: dict[int, tuple[list[int], list[int], list[str]]] = {}

        for entry in syms_data:
            code_id = entry.get("code_id")
            if isinstance(code_id, str) and code_id:
                syms_by_code[_norm_hex(code_id)] = entry
                # Common breakpad form: code_id with a trailing 0.
                syms_by_code[_norm_hex(code_id) + "0"] = entry

            debug_name = entry.get("debug_name")
            if isinstance(debug_name, str) and debug_name:
                syms_by_name[debug_name] = entry

            st = entry.get("symbol_table") or []
            st_sorted = sorted(st, key=lambda x: int(x.get("rva", 0)))
            rvas = [int(x.get("rva", 0)) for x in st_sorted]
            ends = [int(x.get("rva", 0)) + int(x.get("size", 0)) for x in st_sorted]
            names: list[str] = []
            for x in st_sorted:
                si = x.get("symbol", 0)
                if isinstance(si, int) and 0 <= si < len(string_table):
                    names.append(string_table[si])
                else:
                    names.append("UNKNOWN")
            preprocessed[id(entry)] = (rvas, ends, names)

        libs = profile.get("libs") or []

        def match_entry_for_lib(lib: dict[str, Any]) -> dict[str, Any] | None:
            for key in (lib.get("codeId"), lib.get("breakpadId")):
                if isinstance(key, str) and key:
                    k = _norm_hex(key)
                    if k in syms_by_code:
                        return syms_by_code[k]
                    if k.endswith("0") and k[:-1] in syms_by_code:
                        return syms_by_code[k[:-1]]
            for key in (lib.get("debugName"), lib.get("name")):
                if isinstance(key, str) and key and key in syms_by_name:
                    return syms_by_name[key]
            return None

        lib_entries = [match_entry_for_lib(lib) for lib in libs]
        return cls(libs=libs, lib_entries=lib_entries, preprocessed=preprocessed)

    def lookup_symbol(self, lib_index: int | None, rva: int | None) -> str:
        if lib_index is None or rva is None:
            return "UNKNOWN"
        if not (0 <= lib_index < len(self.libs)):
            return "UNKNOWN"

        lib = self.libs[lib_index]
        entry = (
            self.lib_entries[lib_index]
            if 0 <= lib_index < len(self.lib_entries)
            else None
        )
        if entry is None:
            name = lib.get("debugName") or lib.get("name") or f"lib{lib_index}"
            return f"{name} @ 0x{int(rva):x}"

        rvas, ends, names = self.preprocessed[id(entry)]
        i = bisect.bisect_right(rvas, int(rva)) - 1
        if i >= 0 and int(rva) < ends[i]:
            return names[i]

        name = (
            lib.get("debugName")
            or lib.get("name")
            or entry.get("debug_name")
            or f"lib{lib_index}"
        )
        return f"{name} @ 0x{int(rva):x}"


def _iter_samples(
    profile: dict[str, Any],
    symbolicator: Symbolicator,
    weight_mode: str,
) -> tuple[int, dict[str, int], dict[str, int], dict[str, dict[str, int]]]:
    """
    Returns:
      total_weight,
      leaf_counts[name] = weight,
      inclusive_counts[name] = weight,
      leaf_callers[leaf][caller] = weight
    """
    total = 0
    leaf_counts: dict[str, int] = {}
    inclusive_counts: dict[str, int] = {}
    leaf_callers: dict[str, dict[str, int]] = {}

    for thread in profile.get("threads") or []:
        samples = thread.get("samples") or {}
        stacks = samples.get("stack") or []
        sample_weights = samples.get("weight")
        cpu_deltas = samples.get("threadCPUDelta")
        wall_deltas = samples.get("timeDeltas")

        stack_table = thread.get("stackTable") or {}
        stack_prefix = stack_table.get("prefix") or []
        stack_frame = stack_table.get("frame") or []

        frame_table = thread.get("frameTable") or {}
        frame_addr = frame_table.get("address") or []
        frame_func = frame_table.get("func") or []

        func_table = thread.get("funcTable") or {}
        func_resource = func_table.get("resource") or []

        resource_table = thread.get("resourceTable") or {}
        resource_lib = resource_table.get("lib") or []

        symbol_for_frame_cache: dict[int, str] = {}

        def symbol_for_frame(frame_id: int) -> str:
            if frame_id in symbol_for_frame_cache:
                return symbol_for_frame_cache[frame_id]

            if (
                frame_id < 0
                or frame_id >= len(frame_addr)
                or frame_id >= len(frame_func)
            ):
                s = "UNKNOWN"
                symbol_for_frame_cache[frame_id] = s
                return s

            rva = frame_addr[frame_id]
            func_id = frame_func[frame_id]

            lib_index = None
            if isinstance(func_id, int) and 0 <= func_id < len(func_resource):
                resource_id = func_resource[func_id]
                if isinstance(resource_id, int) and 0 <= resource_id < len(
                    resource_lib
                ):
                    lib_index = resource_lib[resource_id]

            s = symbolicator.lookup_symbol(lib_index, rva)
            symbol_for_frame_cache[frame_id] = s
            return s

        # Memoize full stack (symbols) per stack_id to make inclusive counting cheap.
        stack_syms_cache: dict[int, list[str]] = {}

        def stack_symbols(stack_id: int) -> list[str]:
            if stack_id in stack_syms_cache:
                return stack_syms_cache[stack_id]

            syms: list[str] = []
            cur = stack_id
            # Traverse leaf->root via prefix pointers, then reverse.
            while isinstance(cur, int) and cur >= 0 and cur < len(stack_frame):
                frame_id = stack_frame[cur]
                if isinstance(frame_id, int):
                    syms.append(symbol_for_frame(frame_id))
                nxt = stack_prefix[cur] if cur < len(stack_prefix) else None
                if not isinstance(nxt, int) or nxt == -1:
                    break
                cur = nxt

            syms.reverse()
            stack_syms_cache[stack_id] = syms
            return syms

        def weight_at_sample(idx: int) -> int:
            if (
                weight_mode == "cpu"
                and isinstance(cpu_deltas, list)
                and idx < len(cpu_deltas)
            ):
                try:
                    return int(cpu_deltas[idx])
                except Exception:
                    return 0
            if (
                weight_mode == "wall"
                and isinstance(wall_deltas, list)
                and idx < len(wall_deltas)
            ):
                try:
                    # timeDeltas is in ms (float). Store as (ms*1000) int to keep formatting consistent.
                    return int(float(wall_deltas[idx]) * 1000.0)
                except Exception:
                    return 0
            if isinstance(sample_weights, list) and idx < len(sample_weights):
                try:
                    return int(sample_weights[idx])
                except Exception:
                    return 1
            return 1

        for idx, stack_id in enumerate(stacks):
            if not isinstance(stack_id, int):
                continue
            if stack_id < 0 or stack_id >= len(stack_frame):
                continue

            w = weight_at_sample(idx)
            total += w

            # Leaf.
            leaf_frame_id = stack_frame[stack_id]
            if isinstance(leaf_frame_id, int):
                leaf = symbol_for_frame(leaf_frame_id)
                leaf_counts[leaf] = leaf_counts.get(leaf, 0) + w

                # Immediate caller (one frame up).
                caller_name = "(root)"
                if stack_id < len(stack_prefix):
                    caller_stack = stack_prefix[stack_id]
                    if isinstance(caller_stack, int) and caller_stack != -1:
                        if 0 <= caller_stack < len(stack_frame):
                            caller_frame_id = stack_frame[caller_stack]
                            if isinstance(caller_frame_id, int):
                                caller_name = symbol_for_frame(caller_frame_id)

                leaf_callers.setdefault(leaf, {})
                leaf_callers[leaf][caller_name] = (
                    leaf_callers[leaf].get(caller_name, 0) + w
                )

            # Inclusive: every frame on stack.
            # Count each function at most once per sample, so percentages stay <= 100%
            # (this matches the "Samples %" intuition used in PERF.md tables).
            for sym in set(stack_symbols(stack_id)):
                inclusive_counts[sym] = inclusive_counts.get(sym, 0) + w

    return total, leaf_counts, inclusive_counts, leaf_callers


def _format_weight_ms(weight_mode: str, w: int) -> tuple[str, float]:
    if weight_mode == "cpu":
        # µs -> ms
        return f"{(w / 1000.0):,.1f}", w
    if weight_mode == "wall":
        # stored as ms*1000 -> ms
        return f"{(w / 1000.0):,.1f}", w
    return str(int(w)), float(w)


def _write_top_leaves_md(
    path: str,
    label: str,
    weight_mode: str,
    total: int,
    leaf_counts: dict[str, int],
    top_n: int,
) -> None:
    header = (
        "CPU ms"
        if weight_mode == "cpu"
        else ("Wall ms" if weight_mode == "wall" else "Samples")
    )
    items = sorted(leaf_counts.items(), key=lambda kv: kv[1], reverse=True)[:top_n]
    with open(path, "w", encoding="utf-8") as f:
        f.write(f"| # | {header} | % | Leaf |\n")
        f.write("| -: | --: | --: | --- |\n")
        for i, (name, count) in enumerate(items, start=1):
            pct = (count / total * 100.0) if total else 0.0
            v_str, _ = _format_weight_ms(weight_mode, count)
            f.write(f"| {i} | {v_str} | {pct:5.1f}% | {name} |\n")


def _write_top_inclusive_md(
    path: str,
    label: str,
    weight_mode: str,
    total: int,
    inclusive_counts: dict[str, int],
    top_n: int,
) -> None:
    header = (
        "Inclusive CPU ms"
        if weight_mode == "cpu"
        else ("Inclusive Wall ms" if weight_mode == "wall" else "Inclusive Samples")
    )
    items = sorted(inclusive_counts.items(), key=lambda kv: kv[1], reverse=True)[:top_n]
    with open(path, "w", encoding="utf-8") as f:
        f.write(f"| # | {header} | Samples % | Function |\n")
        f.write("| -: | --: | --: | --- |\n")
        for i, (name, count) in enumerate(items, start=1):
            pct = (count / total * 100.0) if total else 0.0
            v_str, _ = _format_weight_ms(weight_mode, count)
            f.write(f"| {i} | {v_str} | {pct:5.1f}% | {name} |\n")


def _write_leaf_callers_md(
    path: str,
    label: str,
    weight_mode: str,
    total: int,
    leaf_counts: dict[str, int],
    leaf_callers: dict[str, dict[str, int]],
    top_leaves_n: int,
    callers_n: int,
) -> None:
    header = (
        "Leaf CPU ms"
        if weight_mode == "cpu"
        else ("Leaf Wall ms" if weight_mode == "wall" else "Leaf Samples")
    )
    top_leaves = sorted(leaf_counts.items(), key=lambda kv: kv[1], reverse=True)[
        :top_leaves_n
    ]
    with open(path, "w", encoding="utf-8") as f:
        f.write(f"| # | {header} | Leaf % | Leaf |\n")
        f.write("| -: | --: | --: | --- |\n")
        for i, (leaf, count) in enumerate(top_leaves, start=1):
            pct = (count / total * 100.0) if total else 0.0
            v_str, _ = _format_weight_ms(weight_mode, count)
            f.write(f"| {i} | {v_str} | {pct:5.1f}% | {leaf} |\n")

        f.write("\n")
        f.write("### Top immediate callers per hot leaf\n\n")

        for leaf, leaf_w in top_leaves:
            f.write(
                f"#### Leaf: `{leaf}` ({_format_weight_ms(weight_mode, leaf_w)[0]} ms, {(leaf_w / total * 100.0 if total else 0.0):.1f}%)\n"
            )
            f.write("| # | Caller CPU ms | Caller % of leaf | Caller |\n")
            f.write("| -: | --: | --: | --- |\n")

            callers = leaf_callers.get(leaf, {})
            items = sorted(callers.items(), key=lambda kv: kv[1], reverse=True)[
                :callers_n
            ]
            for i, (caller, w) in enumerate(items, start=1):
                pct_leaf = (w / leaf_w * 100.0) if leaf_w else 0.0
                v_str, _ = _format_weight_ms(weight_mode, w)
                f.write(f"| {i} | {v_str} | {pct_leaf:5.1f}% | {caller} |\n")
            f.write("\n")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--profile",
        required=True,
        help="Path to samply profile JSON (.json or .json.gz)",
    )
    ap.add_argument(
        "--syms", required=True, help="Path to samply symbols sidecar (.syms.json)"
    )
    ap.add_argument(
        "--out-dir", required=True, help="Output directory for markdown tables"
    )
    ap.add_argument(
        "--label", required=True, help="Label used in output filenames (e.g. rust, zig)"
    )
    ap.add_argument("--weight", default="cpu", choices=["cpu", "wall", "samples"])
    ap.add_argument(
        "--top-n",
        type=int,
        default=60,
        help="Top N functions to output for leaf/inclusive tables",
    )
    ap.add_argument(
        "--callers-n", type=int, default=10, help="Top N immediate callers per hot leaf"
    )
    args = ap.parse_args()

    out_dir = args.out_dir
    os.makedirs(out_dir, exist_ok=True)

    profile = _load_json(args.profile)
    syms = _load_json(args.syms)
    symbolicator = Symbolicator.from_profile_and_syms(profile, syms)

    total, leaf_counts, inclusive_counts, leaf_callers = _iter_samples(
        profile, symbolicator, args.weight
    )

    top_leaves_path = os.path.join(out_dir, f"top_leaves_{args.label}_{args.weight}.md")
    top_inclusive_path = os.path.join(
        out_dir, f"top_inclusive_{args.label}_{args.weight}.md"
    )
    leaf_callers_path = os.path.join(out_dir, f"leaf_callers_{args.label}.md")

    _write_top_leaves_md(
        top_leaves_path, args.label, args.weight, total, leaf_counts, args.top_n
    )
    _write_top_inclusive_md(
        top_inclusive_path, args.label, args.weight, total, inclusive_counts, args.top_n
    )
    _write_leaf_callers_md(
        leaf_callers_path,
        args.label,
        args.weight,
        total,
        leaf_counts,
        leaf_callers,
        top_leaves_n=min(args.top_n, 40),
        callers_n=args.callers_n,
    )

    print("Wrote:")
    print(f"  {top_leaves_path}")
    print(f"  {top_inclusive_path}")
    print(f"  {leaf_callers_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
