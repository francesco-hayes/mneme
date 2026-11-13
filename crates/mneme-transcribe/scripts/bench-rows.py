#!/usr/bin/env python3
"""Turns `mneme-bench --json` reports into rows for docs/benchmarks.md.

    bench-rows.py <folder with <model>-<n>.json reports> <model id>...

Every number printed is taken from the reports; nothing is estimated. A
spread is written as `min / median / max`: over the processes for the load
times, over every pass of every process for the rest.
"""

import glob
import json
import os
import statistics
import sys


def spread(values, digits):
    return " / ".join(
        f"{value:.{digits}f}"
        for value in (min(values), statistics.median(values), max(values))
    )


def grouped(number):
    return f"{number:,}"


def row(folder, model):
    paths = sorted(glob.glob(os.path.join(folder, f"{model}-*.json")))
    if not paths:
        return f"| `{model}` | no reports in {folder} |"
    reports = [json.load(open(path)) for path in paths]

    profiles = {report["tool"]["build_profile"] for report in reports}
    if profiles != {"release"}:
        return f"| `{model}` | not a release build ({', '.join(sorted(profiles))}); not a benchmark |"

    runs = [run for report in reports for run in report["runs"]]
    simulations = [run["simulation"] for run in runs]
    loads = [report["load_average_before"][0] for report in reports if report["load_average_before"]]
    cells = [
        f"`{model}`",
        grouped(reports[0]["model"]["size_bytes"]),
        spread([report["load"]["first_s"] for report in reports], 2),
        spread([report["load"]["second_s"] for report in reports], 2),
        spread([run["real_time_factor"] for run in runs], 4),
        spread([simulation["worst_lag_s"] for simulation in simulations], 2),
        spread([simulation["worst_queued_audio_s"] for simulation in simulations], 2),
        spread([simulation["stop_wait_s"] for simulation in simulations], 2),
        f"{len(reports)} x {len(reports[0]['runs'])}",
        " / ".join(f"{load:.1f}" for load in loads) if loads else "unknown",
    ]
    return "| " + " | ".join(cells) + " |"


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    folder, models = sys.argv[1], sys.argv[2:]
    print(
        "| Model | File size (bytes) | First load (s) | Second load (s) | Real-time factor "
        "| Furthest behind (s) | Most audio waiting (s) | Wait at stop (s) "
        "| Processes x passes | Load average at each start |"
    )
    print("| --- | ---: | --- | --- | --- | --- | --- | --- | --- | --- |")
    for model in models:
        print(row(folder, model))

    # What the chunker did is the same for every model: say it once.
    first = sorted(glob.glob(os.path.join(folder, f"{models[0]}-*.json")))
    if first:
        report = json.load(open(first[0]))
        cutting = report["cutting"]
        print()
        line = f"Recording: {report['audio']['duration_s']:.2f} s. Cut by: {cutting['method']}. "
        line += f"Chunks: {cutting['chunks']}, "
        if cutting["dropped"] is not None:
            line += f"dropped: {cutting['dropped']}, forced cuts: {cutting['forced_cuts']}, "
        line += f"audio sent to the model: {cutting['audio_sent_s']:.1f} s."
        print(line)


if __name__ == "__main__":
    main()
