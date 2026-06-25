#!/usr/bin/env python3
"""Efficiency summary: throughput PER resource (the disruption metric), from *.resources + *.json in a dir.

Usage: summarize-resources.py <results-dir>
Reads each system's `<sys>.resources` (idle + steady-state server RSS/CPU) and its result JSON (achieved
throughput) and prints a Markdown table: at the SAME delivered throughput, who needs how much RAM/CPU — and
the throughput-per-MB-RAM ratio. The OMB client is identical across systems, so this compares the broker/shim
server footprint, which is the honest efficiency axis.
"""
import glob
import json
import os
import statistics
import sys


def read_kv(path):
    d = {}
    for line in open(path):
        if "=" in line:
            k, v = line.strip().split("=", 1)
            d[k] = v
    return d


def throughput_mb(results_dir, sysname):
    # find the JSON whose driver matches (case-insensitive contains)
    for f in glob.glob(os.path.join(results_dir, "*.json")):
        try:
            d = json.load(open(f))
        except (OSError, ValueError):
            continue
        drv = (d.get("driver") or "").lower()
        if sysname.lower() in drv or sysname.lower() in os.path.basename(f).lower():
            rates = [v for v in (d.get("publishRate") or []) if v is not None]
            pr = statistics.mean(rates) if rates else 0.0
            return pr * (d.get("messageSize", 0) or 0) / 1e6
    return 0.0


def main() -> int:
    d = sys.argv[1] if len(sys.argv) > 1 else "."
    files = sorted(glob.glob(os.path.join(d, "*.resources")))
    print("| system | delivered MB/s | idle RSS | load RSS (avg/max) | **anon (non-reclaim)** | load CPU | **MB/s per GB-RAM** |")
    print("|---|---|---|---|---|---|---|")
    rows = []
    for f in files:
        r = read_kv(f)
        s = r.get("system", os.path.basename(f))
        mbps = throughput_mb(d, s)

        def num(k):
            try:
                return float(r.get(k, "0") or 0)
            except ValueError:
                return 0.0

        idle = num("idle_rss_mb")
        load_avg = num("load_rss_mb_avg")
        load_max = num("load_rss_mb_max")
        anon = num("load_anon_mb_max")
        wal = num("wal_dir_mb")
        cpu = num("load_cpu_pct_avg")
        # throughput per GB of RAM the server held under load — the efficiency headline.
        per_gb = mbps / (load_avg / 1024) if load_avg > 0 else 0
        rows.append((s, mbps, idle, load_avg, load_max, cpu, per_gb, anon, wal))
        anon_cell = f"{anon:,.0f} MB" + (f" (+{wal:,.0f} MB WAL on disk, reclaimable)" if wal > 0 else "")
        print(f"| {s} | {mbps:,.0f} | {idle:,.0f} MB | {load_avg:,.0f} / {load_max:,.0f} MB | {anon_cell} | {cpu:.0f}% | **{per_gb:,.0f}** |")

    # if datarail + a broker both present, print BOTH ratios: resident (RSS) and the honest non-reclaimable (anon).
    dr = next((x for x in rows if x[0] == "datarail"), None)
    for x in rows:
        if x[0] != "datarail" and dr and x[3] > 0 and dr[3] > 0:
            ram_ratio = x[3] / dr[3]
            idle_ratio = (x[2] / dr[2]) if dr[2] > 0 else 0
            print(
                f"\n**datarail vs {x[0]} (same harness, ~same delivered rate): "
                f"{ram_ratio:,.1f}× less RAM under load, {idle_ratio:,.1f}× less idle RAM.**"
            )
            if x[7] > 0 and dr[7] > 0:
                anon_ratio = x[7] / dr[7]
                print(
                    f"**Same-ruler (non-reclaimable anon, page cache excluded for BOTH): "
                    f"datarail {dr[7]:,.0f} MB vs {x[0]} {x[7]:,.0f} MB ⇒ {anon_ratio:,.1f}× — the honest "
                    f"scarce-resource number (RAM you must provision; page cache is evictable).**"
                )
    return 0


if __name__ == "__main__":
    sys.exit(main())
