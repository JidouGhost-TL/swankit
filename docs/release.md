# Private release maintenance

Release automation has two manual workflows. Building a candidate does not
create a tag or release. Publishing remains a separate owner-approved action.
Keep the repository private and all Rust crates unpublishable. No registry
upload is performed.

## Build a candidate

After the workflow implementation is reviewed and pushed, dispatch **Release
candidate** on `main`, supplying its current full commit. Older source commits
are intentionally refused: the source, workflow run head and manifest must
identify the same reviewed commit. Dependencies resolve from the committed
registry lockfile; no sibling source checkout or dependency credential is used.

The workflow runs containerized CI and packages the exact installed/tested
wheel with the release CLI, documentation, schemas and notices. A separate
compiler-free Python runtime receives only the packet and synthetic smoke
checker, without build caches or source mounts. The primary CLI is included;
`wg-adapter` and `wg-decode` are source-build auxiliary executables, not part
of this first packet.

The candidate artifact is named `candidate-<source commit>`. Record its run ID,
artifact ID and archive digest from the run summary. Retention is 30 days,
subject to repository policy. Download and retain the complete artifact while
reviewing it. Expired candidates must be rebuilt and qualified again.

`candidate-manifest.json` binds packet files and registry dependencies.
`BUILD-SOURCE.json` binds source files and CLI/wheel bytes. `SHA256SUMS` covers
the offline packet. These internal file hashes are distinct from the GitHub
artifact digest, which covers the transport ZIP downloaded from Actions.

## Qualify the exact wheel privately

Use the candidate source checkout and exact downloaded wheel with the required
serialized corpus runner. Do not replace it with a locally rebuilt wheel or
regenerate frozen expectations. Detailed corpus inputs and logs stay external;
only the compact reviewed approval below belongs in the repository.

Commit a maintainer-reviewed `release-approvals/<candidate-id>.json` on `main`
after independent review and root acceptance. This approval commit is newer
than the qualified source commit; the release tag still targets the latter.
The approval is trusted because it is reviewed repository content, not because
an uploaded file claims success. A minimal schema is:

```json
{
  "schema": "swankit-release-approval/1",
  "accepted": true,
  "independent_review": true,
  "tag": "v0.1.0",
  "source_commit": "FULL_QUALIFIED_COMMIT",
  "lock_sha256": "LOCK_SHA256",
  "manifest_sha256": "MANIFEST_SHA256",
  "run_id": 123,
  "artifact_id": 456,
  "artifact_digest": "sha256:TRANSPORT_ZIP_SHA256",
  "corpus": {
    "exit_code": 0,
    "required": true,
    "write": false,
    "tests_passed": 1,
    "groups_passed": 1,
    "tests_failed": 0,
    "source_commit": "FULL_QUALIFIED_COMMIT",
    "wheel_sha256": "TESTED_WHEEL_SHA256"
  }
}
```

Replace the teaching counts with actual nonzero results and every placeholder
with the measured identity. Missing, failed, empty or mismatched qualification
is refused. Keep machine paths, individual title data and private payloads out
of approval records. Use the same corpus runner infrastructure without
substituting local library sources for the published dependencies.

## Publish approved bytes

After explicit owner approval, dispatch **Publish approved private release**
on `main` with the candidate run/artifact IDs, version tag and approved receipt
path. The read-only job validates repository privacy, successful trusted run,
source/version, artifact expiry and ZIP digest, complete packet hashes and the
corpus receipt. The publication job revalidates everything before writing.
It never executes a program supplied by the candidate artifact.

Only the publication job has release-write permission. It creates an annotated
tag at the qualified source commit, creates a draft, uploads and re-downloads
the exact approved files, then finalizes the release. The complete transport
packet ZIP preserves the offline tree; CLI archive and wheel are also attached
separately. A final read-only job downloads that packet and repeats clean
installation smoke checks.

Tag, draft and asset creation are not atomic. A rerun accepts an exact matching
tag/release and checks existing asset bytes; conflicting targets, bodies,
unexpected assets or changed bytes fail without forced tags or asset deletion.
A published but incomplete release is reported rather than silently repaired.
Investigate a failed run before rerunning it. No rebuild occurs in publication.

Workflow implementation, pushes, candidate uploads and publication are separate
operations. A review of the implementation does not itself authorize a hosted
run, upload, tag or release. Other operating systems, registry publication and
repository visibility changes remain outside this first packet.
