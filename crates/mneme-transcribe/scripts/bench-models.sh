#!/bin/sh
# Measures models for the table in docs/benchmarks.md:
#
#   crates/mneme-transcribe/scripts/bench-models.sh medium large-v3
#   crates/mneme-transcribe/scripts/bench-models.sh --fixed-windows small
#
# For every model id given it runs the release build of `mneme-bench` on
# bench-audio/meeting-sample.wav in three separate processes, three passes
# each, going round the models so that a busy minute does not land on one
# model only. The JSON reports go to bench-audio/results/<date-time>/, and
# at the end it prints one table row per model, made from those reports.
#
# Before the first use:
#
#   crates/mneme-transcribe/scripts/make-bench-audio.sh         # the recording
#   cargo run -p mneme-transcribe --release --bin mneme-models -- fetch <id>
#
# Measure on a quiet machine: check `uptime` first. The load average of
# every process is in its report and in the printed row.
set -eu

crate_dir="$(cd "$(dirname "$0")/.." && pwd)"
repo_dir="$(cd "${crate_dir}/../.." && pwd)"
audio="${crate_dir}/bench-audio/meeting-sample.wav"
bench="${repo_dir}/target/release/mneme-bench"

extra=""
if [ "${1:-}" = "--fixed-windows" ]; then
    extra="--fixed-windows"
    shift
fi
if [ "$#" -eq 0 ]; then
    echo "usage: bench-models.sh [--fixed-windows] <model id>..." >&2
    exit 2
fi
if [ ! -f "${audio}" ]; then
    echo "No recording at ${audio}. Run scripts/make-bench-audio.sh first." >&2
    exit 1
fi

(cd "${repo_dir}" && cargo build -p mneme-transcribe --release --bin mneme-bench)

out="${crate_dir}/bench-audio/results/$(date '+%Y-%m-%d-%H%M%S')"
mkdir -p "${out}"
for round in 1 2 3; do
    for model in "$@"; do
        echo "round ${round}: ${model} (load average$(uptime | sed 's/.*load averages*://'))" >&2
        # shellcheck disable=SC2086
        "${bench}" --model-id "${model}" --audio "${audio}" --runs 3 --json ${extra} \
            > "${out}/${model}-${round}.json"
    done
done

echo "Reports: ${out}" >&2
python3 "${crate_dir}/scripts/bench-rows.py" "${out}" "$@"
