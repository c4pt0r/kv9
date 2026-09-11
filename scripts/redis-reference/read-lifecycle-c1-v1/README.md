# Retained c1 read-lifecycle diagnostic

This directory publishes the executed helpers and
[accepted lifecycle result](../../../docs/LOW-CONCURRENCY-READ-LIFECYCLE.md).
`results.json` contains the independent phase analysis and input hashes;
`readback-summary.json` contains fixture correctness and retention checks.
`artifact-manifest.json` binds the original absolute paths and byte hashes.

The helpers are a retained record of one local execution, not a portable
benchmark launcher. They bind exact source revisions, existing release artifacts,
container IDs and namespace history. The recording driver imports the original
c64 driver only for unchanged constants and source/build preflight; it never
calls the old main or its perf hooks. The two legacy files are included verbatim
for that dependency, with original paths recorded in the manifest. Replay needs
the original source/build/data artifacts and path bindings; missing inputs fail.

The independent reader binds workers and SDK max-in-flight to one. It requires
explicit root recording termination plus fixture readback and all lifecycle
identity/freshness/population/conservation checks. It claims neither CPU profiling
nor throughput acceptance. Preparation documents retain their original
pre-runtime declarations; successful execution is recorded separately.

For a new experiment, derive new helpers and output locations, review and freeze
their identities before recording, and preserve any failure. Do not overlap a
recording with builds, tests, faults, benchmarks or audits. The isolation wrapper
temporarily constrains only its three identity-bound owned containers and
restores their original CPU masks while preserving historical namespace UIDs.
