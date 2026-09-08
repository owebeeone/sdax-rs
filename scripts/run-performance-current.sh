#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
harness_dir=${SDAX_PERFORMANCE_HARNESS:-performance-harness}
manifest="$repo/$harness_dir/Cargo.toml"
source_commit=$(git -C "$repo" rev-parse HEAD)
source_revision=$(python3 -B "$repo/scripts/performance-source-revision.py")
fixture_revision=$(python3 -B "$repo/scripts/performance-fixture-revision.py" --harness "$harness_dir")

out=${1:-"$repo/performance-results/current-$(date -u +%Y%m%dT%H%M%SZ)"}
mkdir -p "$out"
cold_target=$(mktemp -d "${TMPDIR:-/tmp}/sdax-perf-target.XXXXXX")
# Retain build artifacts unless the owner explicitly requests cleanup.
printf '%s\n' "$cold_target" > "$out/retained-target.txt"

{
    echo "measurement_kind=current"
    echo "source_commit=$source_commit"
    echo "source_revision=$source_revision"
    echo "fixture_revision=$fixture_revision"
    echo "harness_dir=$harness_dir"
    echo "captured_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "hostname=$(hostname)"
    echo "uname=$(uname -a)"
    echo "rustc=$(rustc --version --verbose | tr '\n' ';')"
    echo "cargo=$(cargo --version)"
    echo "profile=release(codegen-units=1,lto=thin)"
    echo "runtime=tokio-current-thread"
    echo "workers=1"
    echo "warmup=8"
    echo "timed_samples=40"
    echo "build_samples=20"
    echo "trace=per-workload(default-or-counting-observer)"
} > "$out/metadata.txt"
git -C "$repo" status --short > "$out/source-status.txt"

if command -v lscpu >/dev/null 2>&1; then
    lscpu > "$out/cpu.txt"
elif command -v sysctl >/dev/null 2>&1; then
    sysctl -a 2>/dev/null | grep -E 'machdep.cpu|hw.(model|machine|ncpu|memsize)' > "$out/cpu.txt" || true
fi

cd "$repo"
CARGO_TARGET_DIR="$cold_target" python3 -B scripts/time-command.py "$out/compile-cold.json" -- \
    cargo build --manifest-path "$manifest" --release --locked --offline \
    > "$out/compile-cold.log" 2>&1
CARGO_TARGET_DIR="$cold_target" python3 -B scripts/time-command.py "$out/compile-warm.json" -- \
    cargo build --manifest-path "$manifest" --release --locked --offline \
    > "$out/compile-warm.log" 2>&1

binary="$cold_target/release/sdax-performance-harness"
wc -c < "$binary" | tr -d ' ' > "$out/artifact-bytes.txt"
"$binary" verify > "$out/verify.log" 2>&1
"$binary" bench --samples 40 --warmup 8 --build-samples 20 \
    > "$out/raw-samples.csv" 2> "$out/bench.log"
python3 -B scripts/summarize-performance.py "$out/raw-samples.csv" "$out/summary.csv"

case "$(uname -s)" in
    Darwin)
        probe_counter="macOS /usr/bin/time -l (maximum resident set size in bytes)"
        /usr/bin/time -l -o "$out/resident-probe-native.txt" \
            "$binary" resident-probe --seconds 10 \
            > "$out/resident-probe-stdout.txt" 2> "$out/resident-probe-stderr.txt"
        ;;
    Linux)
        probe_counter="Linux wait4/getrusage RUSAGE_CHILDREN (maximum resident set size in KiB)"
        python3 -B scripts/resident-process-probe.py \
            "$out/resident-probe-native.txt" \
            "$out/resident-probe-stdout.txt" \
            "$out/resident-probe-stderr.txt" \
            "$binary" 10
        ;;
    *)
        echo "resident probe requires the platform runner" >&2
        exit 1
        ;;
esac
{
    echo "boundary=whole optimized harness process including startup, readiness, 10-second resident wait, shutdown, report, and process exit"
    echo "counter_source=$probe_counter"
    echo "cpu_interpretation=user plus system process CPU over the whole wall interval; not pure idle CPU"
    echo "memory_interpretation=native peak resident process memory; not engine-only bytes"
    echo "wakeup_count=unmeasured"
} > "$out/resident-probe-metadata.txt"
echo "$out"
