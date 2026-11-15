#!/usr/bin/env bash
# Invoked only inside the release toolchain container.
set -euo pipefail
[[ ${SWANKIT_CI_CONTAINER:-0} == 1 ]] || exit 64
[[ -z $(git status --porcelain) ]] || { echo 'error: clean source required' >&2; exit 64; }
cargo metadata --locked --format-version 1 > /artifacts/metadata.json
python tools/release.py registry --lock Cargo.lock --metadata /artifacts/metadata.json --output /artifacts/registry.json
# Sourcing retains the exact wheel_run from the installed-wheel test phase.
source tools/ci.sh
cargo build --release --locked -p swankit-cli --bin swankit
wheels=("$wheel_run"/wheels/*.whl)
[[ ${#wheels[@]} == 1 && -f ${wheels[0]} ]] || exit 64
mkdir /artifacts/candidate
python tools/package-release.py --cli "$CARGO_TARGET_DIR/release/swankit" --wheel "${wheels[0]}" \
    --output /artifacts/candidate/packet --source-commit "$(git rev-parse HEAD)"
python -c 'import json, os, platform, subprocess; from pathlib import Path; Path("/artifacts/build-environment.json").write_text(json.dumps({"image":os.environ["SWANKIT_BUILD_IMAGE_ID"],"python":platform.python_version(),"rust":subprocess.check_output(["rustc","-Vv"],text=True)},indent=2))'
