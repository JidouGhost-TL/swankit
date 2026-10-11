#!/usr/bin/env bash
# Build and test an installed abi3 wheel without installing host packages.
set -euo pipefail
mode=${1:-wheel}
case "$mode" in wheel|--ci|--ci-corpus) ;; *) echo 'usage: tools/python-container.sh [--ci|--ci-corpus]' >&2; exit 64 ;; esac
if (($# > 1)); then echo 'too many arguments' >&2; exit 64; fi
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
artifacts=${SWANKIT_PYTHON_ARTIFACTS:?set an external wheel and build artifact directory}
mkdir -p "$artifacts"
artifacts=$(cd -- "$artifacts" && pwd)
if [[ $artifacts == "$root" || $artifacts == "$root/"* ]]; then
    echo 'error: Python artifacts must be outside the repository' >&2
    exit 64
fi
formatkit_dir=${FORMATKIT_DIR:-"$root/../formatkit"}
formatkit_dir=$(cd -- "$formatkit_dir" && pwd)
common_dir=$(git -C "$root" rev-parse --path-format=absolute --git-common-dir)
image=${SWANKIT_PYTHON_IMAGE:-swankit-python:local}
docker build -f "$root/tools/python.Dockerfile" -t "$image" "$root"
mounts=(-v "$root:/workspace/swankit" -v "$formatkit_dir:/workspace/formatkit:ro"
        -v "$artifacts:/artifacts" -v "$common_dir:$common_dir:ro")
runtime_dir=${XDG_RUNTIME_DIR:-/tmp}
mounts+=(-v "$runtime_dir:/runtime-locks")
if [[ -n ${SWANKIT_PROGRESS_LOG:-} ]]; then
    progress_parent=$(cd -- "$(dirname -- "$SWANKIT_PROGRESS_LOG")" && pwd)
    mounts+=(-v "$progress_parent:$progress_parent")
fi
if [[ $mode == --ci-corpus ]]; then
    corpus_dir=${FORMATKIT_CORPUS_DIR:?set an explicit staged corpus directory}
    corpus_dir=$(cd -- "$corpus_dir" && pwd)
    mounts+=(-v "$corpus_dir:$corpus_dir:ro")
    IFS=: read -r -a data_roots <<< "${SWANKIT_CORPUS_DATA_ROOTS:-}"
    python3 "$root/tools/corpus-mounts.py" "$corpus_dir" "${data_roots[@]}" > "$artifacts/corpus-mounts.nul"
    while IFS= read -r -d '' data_directory; do
        mounts+=(-v "$data_directory:$data_directory:ro")
    done < "$artifacts/corpus-mounts.nul"
fi
# Reuse existing pinned formatting/lint components if the Rust base omits them.
container_prefix=/opt
container_path=$container_prefix/python/bin:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
if [[ -n ${SWANKIT_RUST_TOOLCHAIN_DIR:-} ]]; then
    mounts+=(-v "$SWANKIT_RUST_TOOLCHAIN_DIR:$container_prefix/pinned-rust:ro")
    container_path="$container_prefix/pinned-rust/bin:$container_path"
fi
docker run --rm --user "$(id -u):$(id -g)" --memory=4g --cpus=2 \
    "${mounts[@]}" -e USER="$(id -un)" -e LOGNAME="$(id -un)" \
    -e CARGO_HOME=/artifacts/cargo \
    -e CARGO_TARGET_DIR=/artifacts/target -e PATH="$container_path" \
    -e SWANKIT_CI_CONTAINER=1 -e SWANKIT_PYTHON_ARTIFACTS=/artifacts \
    -e SWANKIT_PROGRESS_LOG="${SWANKIT_PROGRESS_LOG:-}" -e XDG_RUNTIME_DIR=/runtime-locks \
    -e FORMATKIT_CORPUS_DIR="${corpus_dir:-}" -e FORMATKIT_CORPUS_REQUIRED=1 \
    -e SWANKIT_INTEROP_BOOTSTRAP="${SWANKIT_INTEROP_BOOTSTRAP:-0}" \
    "$image" bash -euc '
        mode=$1
        if [[ $mode == --ci-corpus ]]; then
            exec tools/ci.sh --corpus
        elif [[ $mode == --ci ]]; then
            exec tools/ci.sh
        fi
        unset FORMATKIT_CORPUS_DIR FORMATKIT_CORPUS_REQUIRED FORMATKIT_CORPUS_WRITE FORMATKIT_CORPUS_RUNNER_LOCKED
        source tools/python-wheel.sh
    ' bash "$mode"
