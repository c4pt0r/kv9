# Read layout diagnostic evidence

See the [report](../READ-LAYOUT-DIAGNOSIS.md). The warm GET-hit gap reproduces;
64-byte function alignment does not fix it. Four first-map snapshots validate
matching within-configuration relative layouts, contents and comparator targets.
The cause remains unresolved; no optimization or database QPS is promoted.

[evidence.tar.gz](evidence.tar.gz) contains 253 members,
66,440,506 expanded bytes and 8,280,302 compressed bytes.
SHA-256: `46a1bd427cfdecd42185b652139c8221daa125e75d734349dc0f58baf8219bb7`. All member sizes and hashes passed readback.
[members.json](members.json) binds each retained input; [manifest.json](manifest.json)
records exclusions and required external inputs.

The exact corpus and qualified dependency sources already exist in the
[outlined mutation packet](../outlined-mutation-path-v1/README.md). Restore its
`outlined/groups.bin` and `outlined/` dependency/source trees at the recorded
paths. The corpus hash is
`d978ba9e49131f6277f975ecd333c33854845cc03bd14ee57a22fff915a08350`.
It is not duplicated here. Ordinary and aligned executables remain in the
retained `/mnt/data` roots with exact hashes in the inspections. Execution
helpers preserve original absolute workspace/dependency paths; restore those
paths or adjust them in a new experiment before rebuilding, never by relabeling
an existing result.

This packet includes the prospective plan, two aligned source/build transactions,
all four exact code inspections, independent preparations, every one of the
24 measurement outputs and 26 terminal records, analysis inputs and statistics.
It also retains four accepted debugger captures, their separate resumed-child
outputs, both rejected decoder attempts and the failed/corrected analysis logs.
Debugger timing is excluded from [analysis.json](analysis.json).
[layout-analysis.json](layout-analysis.json) verifies all 16,384 decoded nodes
against independent state and tree invariants before comparing addresses.

`published-tools/` holds the final tools; `original-tools/` preserves the exact
pre-rename helper bytes. [retained-tool-bindings.json](retained-tool-bindings.json)
explains the inspector namespace correction. It changes no measured Rust,
executable, input plan or sample. [next-engine-diagnostic.json](next-engine-diagnostic.json)
is prospective scope only, not an executed engine comparison. The original
component material/read gates remain failed.
