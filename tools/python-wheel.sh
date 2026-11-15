#!/usr/bin/env bash
# Sourced by container CI so the corpus worker uses the same installed wheel.
set -euo pipefail
if [[ ${SWANKIT_CI_CONTAINER:-0} != 1 ]]; then
    echo 'error: wheel tests require the Python container' >&2
    return 64 2>/dev/null || exit 64
fi
wheel_artifacts=${SWANKIT_PYTHON_ARTIFACTS:?set an external artifact directory}
mkdir -p "$wheel_artifacts"
wheel_artifacts=$(cd -- "$wheel_artifacts" && pwd)
wheel_repo=$(pwd)
if [[ $wheel_artifacts == "$wheel_repo" || $wheel_artifacts == "$wheel_repo/"* ]]; then
    echo 'error: Python artifacts must be outside the repository' >&2
    return 64 2>/dev/null || exit 64
fi
wheel_run=$(mktemp -d "$wheel_artifacts/run.XXXXXXXX")
cargo metadata --locked --all-features --format-version 1 > "$wheel_run/dependency-metadata.json"
python tools/collect-dependency-licenses.py --metadata "$wheel_run/dependency-metadata.json" \
    --lock Cargo.lock --output "$wheel_repo/crates/swankit-py/release-licenses"
maturin build --release --manifest-path crates/swankit-py/Cargo.toml --locked --out "$wheel_run/wheels"
wheel_install="$wheel_run/installed"
python -m pip install --no-index --target "$wheel_install" "$wheel_run"/wheels/*.whl
export PYTHONPATH="$wheel_install${PYTHONPATH:+:$PYTHONPATH}"
python -m pytest crates/swankit-py/tests/test_api.py -q
