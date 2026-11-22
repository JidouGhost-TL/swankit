# Contributing

This workspace uses Rust 1.94.1, edition 2021 and resolver 2. Contributions
are licensed MIT OR Apache-2.0 unless an existing MIT-only port requires MIT;
packages containing or linking those ports declare MIT AND (MIT OR Apache-2.0).
The crates.io release includes the approved library set. Other workspace members
inherit the default unpublished policy. Pushing and publishing require
explicit release approval.
Use the repo-local identity and stage explicit paths; omit authorship trailers.

swankit owns WonderSwan grammars, identity declarations and hardware policy.
Formatkit owns reusable mechanisms. Reuse its checked readers and layout
fields. Propose a mechanism upstream separately only when independent
consumers justify it; never modify the sibling workspace from this work.

Use cited hardware documents and synthetic fixtures. Mesen, MiSTer, ares,
StoicGoose and ARMV30MZ are behavior references only, never implementation
sources. Keep title names in game crates or external corpus specs. No ROMs,
save images, corpus files, machine paths, personal usernames, non-repository
emails or assistant product identifiers may enter the index.

Enable the pre-commit privacy hook:

```sh
git config core.hooksPath .githooks
python3 tools/check-private.py
tools/ci.sh
```

The hook audits indexed blobs; `tools/ci.sh` also audits working files. Review
the contents semantically: pattern checks cannot detect every title name or
secret. Tests use synthetic byte arrays. Set `SWANKIT_PROGRESS_LOG` to the
external job log for workflow phase records. Set `FORMATKIT_CORPUS_DIR` only
for an explicit safe corpus run. `tools/ci.sh --corpus` runs synthetic gates
first, then invokes the sibling safe runner. It refuses corpus write mode.

For later milestones, add an owner crate under `crates/`, inherit workspace
metadata including `publish`, and add its workspace member explicitly.
Depend on the individual pinned Formatkit packages. Declare catalog identities
in their owner crate; compose descriptors in the later product crate. Keep
corpus handlers dev-only and expectations outside Git.
