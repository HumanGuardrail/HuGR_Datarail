#!/usr/bin/env python3
"""Summarize OMB result JSONs into a Markdown table (for the GitHub step summary).

Usage: summarize.py <results-dir>
Reads every *.json TestResult in the dir, prints one Markdown table row per system. Missing/short fields
degrade to 0 rather than crashing — an honest "0" beats a broken summary step.
"""
import glob
import json
import os
import statistics
import sys


def mean(xs):
    xs = [v for v in (xs or []) if v is not None]
    return statistics.mean(xs) if xs else 0.0


def main() -> int:
    results_dir = sys.argv[1] if len(sys.argv) > 1 else "."
    files = sorted(glob.glob(os.path.join(results_dir, "*.json")))
    print("| system | pub rate (msg/s) | pub MB/s | consume rate (msg/s) | E2E p50 (ms) | E2E p99 (ms) | E2E p99.9 (ms) |")
    print("|---|---|---|---|---|---|---|")
    if not files:
        print("| _(no result JSONs produced — see the per-system .out logs in the artifact)_ |||||||")
        return 0
    for f in files:
        try:
            d = json.load(open(f))
        except (OSError, ValueError) as e:
            print(f"| _(unreadable {os.path.basename(f)}: {e})_ |||||||")
            continue
        pr = mean(d.get("publishRate"))
        cr = mean(d.get("consumeRate"))
        ms = d.get("messageSize", 0) or 0
        mb = pr * ms / 1e6
        name = d.get("driver") or d.get("workload") or os.path.basename(f)
        p50 = d.get("aggregatedEndToEndLatency50pct", 0) or 0
        p99 = d.get("aggregatedEndToEndLatency99pct", 0) or 0
        p999 = d.get("aggregatedEndToEndLatency999pct", 0) or 0
        print(f"| {name} | {pr:,.0f} | {mb:,.1f} | {cr:,.0f} | {p50:.2f} | {p99:.2f} | {p999:.2f} |")
    return 0


if __name__ == "__main__":
    sys.exit(main())
