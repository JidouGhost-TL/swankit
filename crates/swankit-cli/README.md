# swankit-cli

Private `swankit` command line binary. It uses the composed product library,
provides human text and `--json`, validates bounded files and definitions, and
writes PNGs only at this edge. Build with `cargo build -p swankit-cli --locked`;
run `swankit --help` or see [docs/cli.md](../../docs/cli.md).

Output paths are required for binary data. Existing outputs require `--force`.
Machine-readable errors are JSON on stderr with a nonzero exit code. Synthetic
smoke tests exercise each command; external corpus tests run only through the
Formatkit safe runner.
