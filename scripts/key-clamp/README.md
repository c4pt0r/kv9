# Safe accessor qualification

Use a fresh `/mnt/data/kv9-work` directory from execution base `b23506d`:

```sh
python3 scripts/key-clamp/prepare.py ROOT
python3 scripts/key-clamp/test.py ROOT
python3 scripts/key-clamp/prove.py ROOT /path/to/pinned/lean
python3 scripts/key-clamp/layout.py ROOT
python3 scripts/key-clamp/audit.py ROOT
```

Preparation copies the previously recorded codegen control and verifies that
only one key-accessor expression differs from the qualified single-buffer
control. Source-checked baseline/control test evidence is reused; their unchanged
tests are not repeated. The changed candidate runs the full engine/common suite,
including the identical generated and payload-replacement models.

The proof rechecks 23 representation statements under the changed key definition
and adds three invariant/accessor lemmas. The allocation probe compiles the exact
candidate and requires all 198 observations to match the earlier control, not
merely an allocation count inferred from the syntax. It emits no elapsed time.

Qualification does not promote the candidate. The separate three-arm performance
tools measure actual release paths against both the failed control and selected
baseline; database correctness, recovery, actual Chaos Mesh and matched Redis
throughput/latency remain required before a production selection.
