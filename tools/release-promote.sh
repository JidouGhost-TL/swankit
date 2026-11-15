#!/usr/bin/env bash
# Only reviewed repository code runs with release-write permission.
set -euo pipefail
mode=${1:?validate or publish}
[[ $mode == validate || $mode == publish ]] || exit 64
[[ ${GITHUB_REF:-} == refs/heads/main ]] || exit 64
[[ ${RUN_ID:-} =~ ^[1-9][0-9]*$ && ${ARTIFACT_ID:-} =~ ^[1-9][0-9]*$ ]] || exit 64
[[ ${RELEASE_TAG:-} =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || exit 64
[[ ${RECEIPT:-} =~ ^release-approvals/[A-Za-z0-9_-]+\.json$ ]] || exit 64
root=$(git rev-parse --show-toplevel)
scratch=${RUNNER_TEMP:?runner scratch required}/release-promotion
mkdir -p "$scratch"
args=()
[[ $mode == validate ]] || args+=(--publish)
docker run --rm --network bridge --read-only --user "$(id -u):$(id -g)" \
    --mount type=bind,src="$root",dst=/trusted,readonly \
    --mount type=bind,src="$scratch",dst=/job \
    -e GH_TOKEN python:3.13-slim@sha256:d3449c0493345868b09708d7056dce5edda3a75233ea86008fc80c934d355a10 python -B /trusted/tools/release.py promote \
    --receipt "/trusted/$RECEIPT" --repository "$REPOSITORY" --run-id "$RUN_ID" \
    --artifact-id "$ARTIFACT_ID" --tag "$RELEASE_TAG" --destination /job/candidate \
    --output /job/result.json "${args[@]}"
