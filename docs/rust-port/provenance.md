# Rust port provenance gate

This is an engineering record, not legal advice.

The repository README identifies the current C++ tree as partly derived from
the unauthorized 2018 TF2 leak. `LICENSE` is the Source 1 SDK license and
authorizes use and free redistribution of that SDK and modifications to it; it
does not establish that every file currently in this repository came from the
SDK or another authorized source.

Consequently, public distribution is blocked until each shipping source file
has an auditable authorized origin or has been replaced with a clean
implementation. New Rust code must be written from public format behavior,
authorized SDK headers/sources, and black-box compatibility observations. It
must not be copied or mechanically translated from a file whose provenance has
not been cleared.

The locally installed Steam game may be used as a legally held black-box
compatibility corpus. Game assets are never copied into this repository; only
relative names, sizes, and cryptographic fingerprints may be recorded.

`scripts/verify_rust_port_revisions.py` checks the repository and submodule
commits recorded in `revisions.toml`; CI runs it before the Rust compatibility
tests. Pass `--require-clean` when capturing release evidence from a dedicated
checkout. Revision verification makes the oracle reproducible but does not, by
itself, clear any source's legal origin.

Release evidence required to close this gate:

1. A per-file provenance inventory for every source linked into shipping
   binaries.
2. Replacement or removal of every item without an authorized origin.
3. License and third-party notice review for the resulting distribution.
4. A clean-tree build showing that no quarantined source enters the product.
