#!/usr/bin/env bash
# Download and execute the published packet only in a read-only-token job.
set -euo pipefail
[[ ${GITHUB_REF:-} == refs/heads/main ]] || exit 64
[[ ${RELEASE_TAG:-} =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || exit 64
[[ ${RECEIPT:-} =~ ^release-approvals/[A-Za-z0-9_-]+\.json$ ]] || exit 64
root=$(git rev-parse --show-toplevel)
scratch=${RUNNER_TEMP:?runner scratch required}/release-verify
mkdir -p "$scratch"
image=python:3.13-slim@sha256:d3449c0493345868b09708d7056dce5edda3a75233ea86008fc80c934d355a10
docker run --rm --network bridge --read-only --user "$(id -u):$(id -g)" \
    --mount type=bind,src="$root",dst=/trusted,readonly --mount type=bind,src="$scratch",dst=/job \
    -e GH_TOKEN "$image" python -B /trusted/tools/release.py download-release \
    --receipt "/trusted/$RECEIPT" --repository "$REPOSITORY" --tag "$RELEASE_TAG" \
    --destination /job/downloaded --output /job/download-check.json
mkdir "$scratch/smoke"
docker run --rm --network none --read-only --user "$(id -u):$(id -g)" \
    --mount type=bind,src="$scratch/downloaded/packet",dst=/packet,readonly \
    --mount type=bind,src="$scratch/smoke",dst=/scratch \
    --mount type=bind,src="$root/tools/release-smoke.py",dst=/check.py,readonly \
    "$image" python -B /check.py --packet /packet --output /scratch/check.json
