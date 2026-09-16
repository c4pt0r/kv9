# Outlined mutation experiment evidence

See the [report](../OUTLINED-MUTATION-PATH.md). The named inline adapter failed
the codegen gate and was not timed. The real shared-clone extraction passes its
scoped correctness qualification and improves every write mean in both orders,
but fails the declared material/read gates. Production is unchanged.

[evidence.tar.gz](evidence.tar.gz) retains 2,805 files,
167,124,238 expanded bytes and 35,078,306 compressed bytes.
SHA-256: `7b17d2f6bfc8a8a487f71b9b5020cbde0b888cdfc94a16b02078e229348a1c7c`. Every member passed size/SHA-256 readback.
[manifest.json](manifest.json) records exclusions and [members.json](members.json)
binds each source path, size and digest. The first publication draft remains
in `/mnt/data/kv9-work/outlined-mutation-packet-first-20260916`; the final packet
omits only the redundant named-harness corpus copy.

The archive separates `named/` and `outlined/` attempts. It contains dependency
sources/patches/licenses, proof inputs and rejected controls, source/build/cache
identities, successful and failed build attempts, full codegen, engine smokes,
the prospective timing plan, every raw timing/count row and all 338 process
terminal records. Restore `named/groups.bin` from the identical
`outlined/groups.bin` if reproducing the named harness. That harness's timing
gate remains closed. Executables and compiled Lean artifacts stay in the
original retained roots with their hashes recorded; current progress and
GitHub body snapshots are not source evidence.

[analysis.json](analysis.json) reports all 66 cells and the unchanged failed
acceptance rule. [qualification-audit.json](qualification-audit.json) and
[codegen-decision.json](codegen-decision.json) establish permission for component
timing only. They do not establish runtime, recovery or Chaos Mesh promotion.
The read-codegen review and exact assembly are retained inside the archive;
operand-shape similarity is not a causal explanation of the measured regression.

Upstream source is accompanied by its original [archery MIT license](ARCHERY-LICENSE.md),
[rpds MIT license](RPDS-LICENSE.md), and triomphe [MIT](TRIOMPHE-LICENSE-MIT) /
[Apache-2.0](TRIOMPHE-LICENSE-APACHE) licenses.
