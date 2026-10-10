#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Optional external gate: supplied, independently built C versus the Rust core.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
profile=${1:-}
if [[ $# != 1 || ($profile != upstream && $profile != revival) || -z ${WG_C_ADAPTER:-} ]]; then
    echo 'usage: WG_C_ADAPTER=compiled-wonderfence tools/wg-differential.sh upstream|revival' >&2
    exit 64
fi
if [[ ! -x $WG_C_ADAPTER ]]; then echo 'error: WG_C_ADAPTER must be executable' >&2; exit 66; fi
unset FORMATKIT_CORPUS_DIR FORMATKIT_CORPUS_REQUIRED FORMATKIT_CORPUS_WRITE FORMATKIT_CORPUS_RUNNER_LOCKED
cd "$root"
if [[ -z ${WG_RUST_ADAPTER:-} ]]; then
    export CARGO_BUILD_JOBS=1
    cargo build -p ws-wondergate --example wg_differential --locked
    target=${CARGO_TARGET_DIR:-target}
    rust_adapter="$target/debug/examples/wg_differential"
else
    rust_adapter=$WG_RUST_ADAPTER
fi
if python3 tools/wg_differential.py --c "$WG_C_ADAPTER" --rust "$rust_adapter" \
    --profile "$profile" --streams "${WG_STREAMS:-10000}" --seed "${WG_SEED:-0x13c0ffee}"; then
    if [[ -n ${SWANKIT_PROGRESS_LOG:-} ]]; then
        printf '%s | differential | %s profile: %s seeded streams passed, identical stdout | continue sign-off gates\n' "$(date -Iminutes)" "$profile" "${WG_STREAMS:-10000}" >> "$SWANKIT_PROGRESS_LOG"
    fi
else
    status=$?
    if [[ -n ${SWANKIT_PROGRESS_LOG:-} ]]; then
        printf '%s | differential failed | %s profile: exit %s | investigate; stop if cause repeats\n' "$(date -Iminutes)" "$profile" "$status" >> "$SWANKIT_PROGRESS_LOG"
    fi
    exit "$status"
fi
