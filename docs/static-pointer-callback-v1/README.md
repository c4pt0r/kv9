# Static pointer callback qualification evidence

See the [report](../STATIC-POINTER-CALLBACK.md) for scope, results and remaining
performance/runtime gates. No production promotion or database speedup is claimed.

The [archive](evidence.tar.gz) retains 519 files (29939817 expanded bytes)
from the isolated qualification, including tests, proof inputs/controls, exact
ordinary release source/build records, disassembly, semantic smokes, independent
audit and source preparation readback. Every member passed SHA-256/size readback.

Archive SHA-256: `7255c8433786c71f17951f96dee0821663ed122ec9383a4cf53ba5d419ab2928`.

The [manifest](manifest.json) records archive identity and exclusions;
[members.json](members.json) binds every retained file. Executables remain local,
with their identities recorded. The original corpus is the previously retained
`groups.bin`, SHA-256
`d978ba9e49131f6277f975ecd333c33854845cc03bd14ee57a22fff915a08350`.

Source crate snapshots retain their original MIT license files. The proof is
conditional on the explicit Rust/library/compiler assumptions documented in the
report. Failed proof mutations are expected rejecting controls, not accepted
proofs. Smoke timestamps are not a performance screen.
