#!/usr/bin/env bash
# Synthetic gates, followed by an optional externally rooted serialized corpus run.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
case ${1:-} in
    '') corpus=0 ;;
    --corpus) corpus=1 ;;
    *) echo 'usage: tools/ci.sh [--corpus]' >&2; exit 64 ;;
esac
if (($# > 1)); then echo 'too many arguments' >&2; exit 64; fi
corpus_dir=${FORMATKIT_CORPUS_DIR:-}
if [[ ${FORMATKIT_CORPUS_WRITE:-0} != 0 ]]; then
    echo 'error: CI never regenerates frozen corpus specs' >&2
    exit 64
fi
log_phase() {
    if [[ -n ${SWANKIT_PROGRESS_LOG:-} ]]; then
        printf '%s | %s | %s | %s\n' "$(date -Iminutes)" "$1" "$2" "$3" >> "$SWANKIT_PROGRESS_LOG"
    fi
}
gate() {
    local phase=$1
    shift
    if "$@"; then
        log_phase "$phase" '1 check passed' 'continue CI'
    else
        local status=$?
        log_phase "$phase" "1 check failed, exit $status" 'investigate; stop if cause repeats'
        return "$status"
    fi
}
if [[ ${SWANKIT_INTEROP_BOOTSTRAP:-0} != 0 ]]; then
    echo 'error: interop bootstrap is forbidden in CI' >&2
    exit 64
fi
# Python binding gates use the pinned container; host invocations delegate all
# gates so the corpus worker always imports the freshly tested wheel.
if [[ ${SWANKIT_CI_CONTAINER:-0} != 1 ]]; then
    artifacts=${SWANKIT_PYTHON_ARTIFACTS:?set an external wheel and build artifact directory}
    if ((corpus)); then
        export FORMATKIT_CORPUS_DIR="$corpus_dir"
        exec "$root/tools/python-container.sh" --ci-corpus
    fi
    exec "$root/tools/python-container.sh" --ci
fi
export CARGO_BUILD_JOBS=1
unset FORMATKIT_CORPUS_DIR FORMATKIT_CORPUS_REQUIRED FORMATKIT_CORPUS_WRITE FORMATKIT_CORPUS_RUNNER_LOCKED
gate privacy python3 tools/check-private.py --worktree
gate privacy-tests python3 -m unittest discover -s tools/tests -v
gate fmt cargo fmt --all -- --check
gate clippy cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
gate synthetic cargo test --workspace --all-features --locked
gate no-default-features cargo build --workspace --no-default-features --locked
gate cli-non-saves cargo test -p swankit-cli --no-default-features --locked --test smoke
gate python-wheel source tools/python-wheel.sh
if ((corpus)); then
    if [[ -z $corpus_dir || ! -d $corpus_dir ]]; then
        log_phase corpus '0 samples; invalid explicit root' 'set FORMATKIT_CORPUS_DIR'
        echo 'error: --corpus requires FORMATKIT_CORPUS_DIR' >&2
        exit 66
    fi
    formatkit_dir=${FORMATKIT_DIR:-"$root/../formatkit"}
    export FORMATKIT_CORPUS_DIR="$corpus_dir" FORMATKIT_CORPUS_REQUIRED=1
    gate corpus "$formatkit_dir/tools/run-corpus-safe.sh" --workspace --all-features --locked --release --test corpus
fi
log_phase ci 'all requested gates passed' 'review and commit explicit paths'
