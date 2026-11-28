# swankit

Rust library for WonderSwan cartridge metadata, graphics, compression, text,
sound, patches and saves. `Toolkit::new` validates the Formatkit module catalogue.
`catalog` exposes support metadata, `detect` probes input bytes and
`info` returns cartridge metadata. Typed extraction,
rendering and IPS operations dispatch through the composed module identities
and call the existing owner APIs. File I/O, text presentation and PNG writing
belong to the CLI.

The [Swankit wiki guide](https://jidoughost-tl.github.io/wonderswan-wiki/guides/swankit)
covers setup, save editing and asset workflows.

```rust
let toolkit = swankit::Toolkit::new()?;
let found = toolkit.detect(b"PATCHEOF", None);
assert_eq!(found.selected.as_deref(), Some("ws-patch-ips"));
# Ok::<(), formatkit_core::Error>(())
```

Headerless formats contribute `EmbeddedFormatSupport` rows with explicit
selection requirements. No extension or plausible decoding creates a global
codec, graphics, text or sound match. Extraction uses versioned JSON definitions.
ROMs, save images and per-ROM asset maps are supplied by the caller.

Runtime rows use the legacy metadata state because the owner APIs return typed
Rust values rather than Formatkit namespace/leaf callback contracts. They do
not advertise strict executable runtime callbacks or writer proof contracts.
The product's typed methods are the executable dispatch surface. Promoting
those contracts is a future separate owner change, not a Formatkit edit.

Synthetic tests cover composition, detection, extraction and malformed input.
Dev-only corpus tests check external frozen owner facts through the serialized
safe runner. Expectations and population counts remain outside Git.

The default `saves` feature composes ws-save and ws-saves too.
Registry distributions include the public definition catalogue. Media detection
reports possible sizes and physical interpretations; definition candidates use
positive program validity/checksum flags, excluding declared working regions.
`detect_with_save_context` accepts explicit logical names, lookup tables and
companion images. Shared sizes and signatures may yield several candidates;
`auto_save` requires exactly one. Typed decode, stored-field edits and repair
call the owner engine and keep output in memory until the edge writes it.
