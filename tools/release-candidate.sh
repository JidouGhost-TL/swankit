#!/usr/bin/env bash
# Build only in the existing pinned toolchain container, without sibling sources.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
artifacts=${SWANKIT_RELEASE_ARTIFACTS:?set a fresh external artifact directory}
run_id=${SWANKIT_RELEASE_RUN_ID:?set the candidate workflow run identifier}
mkdir -p "$artifacts"
artifacts=$(cd -- "$artifacts" && pwd)
if [[ $artifacts == "$root" || $artifacts == "$root/"* || -e $artifacts/candidate ]]; then
    echo 'error: candidate output must be fresh and external' >&2
    exit 64
fi
[[ $run_id =~ ^[1-9][0-9]*$ ]] || exit 64
image=swankit-release-candidate:local
docker build -f "$root/tools/python.Dockerfile" -t "$image" "$root"
image_id=$(docker image inspect --format '{{.Id}}' "$image")
common_dir=$(git -C "$root" rev-parse --path-format=absolute --git-common-dir)
docker run --rm --user "$(id -u):$(id -g)" --cpus 2 --memory 4g \
    --mount type=bind,src="$root",dst=/workspace/swankit \
    --mount type=bind,src="$common_dir",dst="$common_dir",readonly \
    --mount type=bind,src="$artifacts",dst=/artifacts \
    -e USER=jidoughost -e LOGNAME=jidoughost -e CARGO_HOME=/artifacts/cargo \
    -e CARGO_TARGET_DIR=/artifacts/target -e CARGO_BUILD_JOBS=1 \
    -e SWANKIT_CI_CONTAINER=1 -e SWANKIT_PYTHON_ARTIFACTS=/artifacts/python \
    -e SWANKIT_RELEASE_RUN_ID="$run_id" -e SWANKIT_BUILD_IMAGE_ID="$image_id" \
    --workdir /workspace/swankit "$image_id" bash tools/release-build.sh
# Exercise only installed packet files in a second source-free container.
mkdir "$artifacts/smoke"
docker run --rm --network none --user "$(id -u):$(id -g)" \
    --mount type=bind,src="$artifacts/candidate/packet",dst=/packet,readonly \
    --mount type=bind,src="$artifacts/smoke",dst=/scratch \
    --mount type=bind,src="$root/tools/release-smoke.py",dst=/check.py,readonly \
    python:3.13-slim@sha256:d3449c0493345868b09708d7056dce5edda3a75233ea86008fc80c934d355a10 \
    python /check.py --packet /packet --output /scratch/packet-smoke.json
docker run --rm --network none --user "$(id -u):$(id -g)" \
    --mount type=bind,src="$root",dst=/workspace/swankit,readonly \
    --mount type=bind,src="$common_dir",dst="$common_dir",readonly \
    --mount type=bind,src="$artifacts",dst=/artifacts \
    --workdir /workspace/swankit "$image_id" \
    python tools/release.py manifest --packet /artifacts/candidate/packet --source /workspace/swankit \
    --run-id "$run_id" --registry /artifacts/registry.json --smoke /artifacts/smoke/packet-smoke.json \
    --environment /artifacts/build-environment.json \
    --output /artifacts/candidate/candidate-manifest.json
