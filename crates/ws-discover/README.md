# ws-discover

Evidence-driven, verified draft asset maps. The input is a ROM, a raw snapshot
seed, work RAM and I/O ports, runtime source observations and optional external
text profiles. See [asset families and pipeline limitations](../../docs/asset-families.md).

The core probes explicitly bounded codecs and accepts a compressed payload only
when its entire useful decoded output equals captured RAM. It verifies the final
map, keeps references incomplete, and reports rejected candidates and coverage.
Collection adapters live in `tools/discover-assets.py` and `tools/discover-raw.py`.
Family profiles are data; no title identities are embedded in generic code.

## ROM-only format search

`search::search(bytes, &search::Options::default())` returns bounded, ranked
format leads without runtime evidence. The CLI exposes single-file and resumable
parallel manifest searches. This is separate from `discover`, which consumes
explicit evidence and verifies draft maps. Search leads are not accepted assets;
see [format search](../../docs/format-search.md).
