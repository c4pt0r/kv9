# Retained direct peer body c1/c64 screening

The first 24-cohort recording and independent audit pass; the candidate is
**rejected for performance**. All eight throughput comparisons regress and
mean latency worsens in all eight. See
[the report](../../../docs/DIRECT-PEER-BODY-SCREENING.md),
[selection.json](selection.json), [statistics.json](statistics.json) and
[the complete readout](READOUT.md).

`inventory.json` records exact original paths, sizes and SHA-256 hashes for
the retained helpers and results. Runnable files retain their executed bytes
and absolute checkout/build/helper dependencies. This is an evidence bundle,
not a path-independent replacement for those dependencies. Prepare the pinned
sources/builds and original helper paths before reproduction; use new output
directories and preserve earlier attempts.

Source candidate: `/tmp/kv9-direct-peer-body`, revision
`6707bcccf15ea788ff231f4263f43a9f73fa63dd`.
Accepted control: `/tmp/kv9-rpc-worker-pair`, revision
`5ee897a2f58c57bdf17ea1757c96224adf0f0dbb`.

The root-owned recording command was:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 \
  taskset -c 6-15,22-31 /usr/bin/python3 \
  /tmp/kv9-direct-peer-body-comparison-preparation/isolate-and-run.py \
  --driver /tmp/kv9-direct-peer-body-comparison-preparation/matched-driver.py \
  --output /tmp/kv9-direct-peer-body-matched-diagnostic-first \
  --driver-arguments-file /tmp/kv9-direct-peer-body-comparison-preparation/driver-arguments.json
```

The frozen audit used the recording's `cohorts` directory with
`--root-session 11757 --root-exit-code 0 --successful-arms 24` and driver SHA
`7be1ffee291a28c7500f6e1ad58f5f6dea34befcf69eff2d48c56db86796425a`.
`audit-invocation-first.json` retains its exact argument vector and bound bytes.
The result is `audit.json`; the complete raw-input inventory remains in the
original `results-first/input-inventory.json`.

Six driver/client contract tests and ten independent auditor controls ran
before the smoke and timing. The separate 12-cohort smoke has 10,605,645
successful calls and 40 exited lifetimes; its timing is excluded. Source
validation, process E2E and their scopes are retained in the corresponding
JSON files and review note. `preparation-notes.json` and
`control-correction.json` preserve preparation/failure-path corrections without
relabeling a failed run.

All routine checks were local. No GitHub workflow was dispatched. The rejected
candidate was not advanced to broad workspace/Chaos acceptance.
