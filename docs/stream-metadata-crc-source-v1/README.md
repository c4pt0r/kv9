# Stream metadata on CRC: source validation

Candidate `bb13e4313c313ca910969d1f01ef862619b57906` integrates the original
immutable authorization metadata adapter on the selected CRC runtime.

The source passes 228 default and 238 experimental server tests/doctests,
with one ignored in each overlapping configuration. These are not full-workspace
counts. Formatting and all-target experimental server Clippy pass with warnings
denied. Source session 66503 terminates with exit 0 (tool e35172).
The six Rust adapter files match original94d8b9f. The five Raw handlers,
authentication/context readers and admission method match the original
metadata-independent consumer contract. The first static reader misspelled the
generic authentication function delimiter; its failure record is preserved.
No tests failed or were repeated to address that reader error.

All 596 tested source files match the committed source. Test snapshots preserve
their precommit parent/dirty state. The clean default release is fresh after
first-party cache invalidation: 11 first-observed compiled units, 20 artifact
rows. Release session69882 terminates with exit0 (efd511), and original source,
manifests, binary hashes and compilation inputs are checked. The server SHA256
is `2ccd4d8bb7dde89418cbe3a4929df0f072af5fedf5f9da69678d0b91ec3cd3ca`.

This archive retains 39 exact files (1,036,147 decoded
bytes, 188,688 compressed bytes). Native binaries remain local; inventories
bind them by hash. No process recovery, new performance or exact-source Chaos
acceptance is claimed. The earlier isolated adapter's acceptance does not
transfer to this combination. Current CRC remains selected.

Run `python3 docs/stream-metadata-crc-source-v1/verify.py` for byte integrity
only; it neither extracts the archive nor executes correctness tests.
