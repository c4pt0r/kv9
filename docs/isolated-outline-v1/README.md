# Isolated shared-clone qualification evidence

See the [report](../ISOLATED-OUTLINE-QUALIFICATION.md). Source, conditional proof,
release codegen and correctness qualification pass. No timing or production
promotion is included; the next screen remains prospective.

[evidence.tar.gz](evidence.tar.gz) contains 665 members,
52,394,240 expanded bytes and 7,347,199 compressed bytes.
SHA-256: `f449e30cb7dd14299e358eba94679e40c418dfc71d332a511bdbe0f4278427a3`.
Every member size/hash passed readback. [members.json](members.json) and
[manifest.json](manifest.json) record all retained bytes and exclusions.

The packet contains exact registry and candidate dependency sources, both release
source/build transactions and disassemblies, 29 conditional proof checks / 14
rejecting controls, 523 passed tests / three existing ignored tests, and both
independently validated semantic preparations. The failed initial engine fixture
metadata and its corrected continuation are both retained. Source copies and
logs identify original registry archery in actual release and engine/rpds tests;
the archery-specific fixture also includes retained test-only callback cases.

The original corpus is already published as `outlined/groups.bin` in the
[outlined packet](../outlined-mutation-path-v1/README.md), SHA-256
`d978ba9e49131f6277f975ecd333c33854845cc03bd14ee57a22fff915a08350`. The inherited proof checker also uses
that packet's original combined-candidate source tree for dependency lemmas;
the isolated composition binds original archery separately. Archive checksums
for upstream `.crate` inputs are in `qualification-plan.json`. Compiled binaries
remain local, with exact hashes in their records.

Reproduction preserves recorded absolute paths. Restore them or adapt a fresh
experiment before building; never relabel an existing result. The
[next screen plan](next-performance-plan.json) is not executed evidence.
No MinIO, ordinary recovery, actual Chaos, database QPS or Redis acceptance is
claimed by this packet.
