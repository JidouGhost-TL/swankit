#!/usr/bin/env bash
# Build and test an installed abi3 wheel without installing host packages.
set -euo pipefail
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
# Reuse existing pinned formatting/lint components if the Rust base omits them.
container_path=/opt/python/bin:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
if [[ -n ${SWANKIT_RUST_TOOLCHAIN_DIR:-} ]]; then
    mounts+=(-v "$SWANKIT_RUST_TOOLCHAIN_DIR:/opt/pinned-rust:ro")
    container_path="/opt/pinned-rust/bin:$container_path"
fi
docker run --rm --user "$(id -u):$(id -g)" --memory=4g --cpus=2 \
    "${mounts[@]}" -e USER="$(id -un)" -e LOGNAME="$(id -un)" \
    -e HOME=/artifacts/home -e CARGO_HOME=/artifacts/cargo \
    -e CARGO_TARGET_DIR=/artifacts/target -e PATH="$container_path" \
    "$image" bash -euc '
        unset FORMATKIT_CORPUS_DIR FORMATKIT_CORPUS_REQUIRED FORMATKIT_CORPUS_WRITE FORMATKIT_CORPUS_RUNNER_LOCKED
        maturin build --release --manifest-path crates/swankit-py/Cargo.toml --locked --out /artifacts/wheels
        install_dir=$(mktemp -d)
        python -m pip install --no-index --target "$install_dir" /artifacts/wheels/*.whl
        PYTHONPATH="$install_dir" python -m pytest crates/swankit-py/tests/test_api.py -q
    '
