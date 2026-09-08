#!/usr/bin/env python3
"""Summarize raw benchmark samples without third-party packages."""

import csv
import math
import statistics
import sys
from collections import defaultdict


def percentile(values: list[int], fraction: float) -> int:
    ordered = sorted(values)
    return ordered[max(0, math.ceil(fraction * len(ordered)) - 1)]


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: summarize-performance.py RAW.csv SUMMARY.csv", file=sys.stderr)
        return 2
    groups: dict[tuple[str, str, int, int], list[dict[str, str]]] = defaultdict(list)
    with open(sys.argv[1], newline="", encoding="utf-8") as handle:
        for row in csv.DictReader(handle):
            key = (
                row["phase"],
                row["workload"],
                int(row["nodes"]),
                int(row["edges"]),
            )
            groups[key].append(row)

    fields = [
        "phase",
        "workload",
        "nodes",
        "edges",
        "samples",
        "median_ns",
        "p95_ns",
        "min_ns",
        "max_ns",
        "allocations",
        "allocated_bytes",
    ]
    with open(sys.argv[2], "w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields)
        writer.writeheader()
        for key in sorted(groups):
            rows = groups[key]
            nanos = [int(row["nanos"]) for row in rows if int(row["nanos"]) > 0]
            allocations = [int(row["allocations"]) for row in rows]
            allocated_bytes = [int(row["allocated_bytes"]) for row in rows]
            writer.writerow(
                {
                    "phase": key[0],
                    "workload": key[1],
                    "nodes": key[2],
                    "edges": key[3],
                    "samples": len(nanos),
                    "median_ns": int(statistics.median(nanos)) if nanos else "",
                    "p95_ns": percentile(nanos, 0.95) if nanos else "",
                    "min_ns": min(nanos) if nanos else "",
                    "max_ns": max(nanos) if nanos else "",
                    "allocations": max(allocations),
                    "allocated_bytes": max(allocated_bytes),
                }
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
