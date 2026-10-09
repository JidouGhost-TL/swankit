# swankit-py

Private PyO3 bindings for the composed Rust API, using the Python 3.11 stable
ABI. The installed package is `swankit`; Rust owner objects remain authoritative.
See [Python usage and container testing](../../docs/python.md).

Build artifacts and frozen corpus expectations remain outside the repository.
Synthetic Python tests exercise the installed wheel. The `corpus` integration
test uses Formatkit's serialized harness and requires an installed wheel in the
container; without an explicit corpus root it skips normally.
