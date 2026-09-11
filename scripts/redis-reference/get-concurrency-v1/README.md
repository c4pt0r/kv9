# Retained single-GET concurrency diagnostic v1

These are the exact executed driver, independently authored auditor, isolation
wrapper, configurations and pre-runtime contract tests for
[the accepted diagnostic](../../../docs/GET-CONCURRENCY-CURVE.md).
`results.json` publishes all 24 cohort populations and evidence hashes.

This directory is an immutable record of a specific local diagnostic, not a
portable benchmark launcher. The helpers bind source revisions, release files,
owned container IDs, namespace history and absolute artifact paths. The auditor
also binds the original `/tmp/kv9-get-concurrency-preparation/matched-driver.py`
invocation. Preserve those bytes and original evidence to replay the audit.
Missing inputs fail rather than silently selecting another source or fixture.

For a new experiment, derive a new preparation and output directory, review
the exact source/resource bindings and freeze its protocol before smoke or
timing. Do not rerun into an existing output, reuse this timing's verdict for
a different build, or report refused calls as useful throughput. Keep timing
separate from builds, tests, fault injection, profiling and audits. The wrapper
changes CPU masks only for its three identity-checked owned containers and
restores them on completion; it preserves historical namespace identities.

`protocol.json` is the original pre-runtime declaration. Its
`runtime_executed: false` records its creation phase; completed execution is
documented in `results.json`, not by rewriting that frozen declaration.
