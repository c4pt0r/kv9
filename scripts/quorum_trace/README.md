# Local quorum trace reader

Use this only with the opt-in diagnostic branch. The input is one process's
`quorum-trace.json`; expected identity comes from independent process/status
binding, not the input itself. All timestamps stay within that process.

```sh
python3 scripts/quorum_trace/read_trace.py --input /path/quorum-trace.json --identity /path/expected-identity.json --output /path/new-analysis.json
```

The exact identity keys are node_id, process_id, exporter_created_unix_ns,
process_start_ticks and boot_id. Both OS identity fields are required for
accepted attribution. Unix nanoseconds are decimal strings.

Run finite synthetic controls with `python3 scripts/quorum_trace/test_read_trace.py`.
The 17 controls passed in the retained preparation; these two Python files are
copied byte-for-byte. Missing intermediates leave a context incomplete even
when an exact observed endpoint span remains available. Any trace loss disables
context attribution; duplicate contexts, routes, terms, counters and reversed
spans retain explicit unknown/ambiguous accounting. Do not infer a quorum-closing
response or network-only latency.

See `docs/QUORUM-MESSAGE-TRACE.md` and the frozen reader evidence under
`docs/quorum-trace-source-v1/reader/`. No actual timing is accepted by synthetic
controls alone.
