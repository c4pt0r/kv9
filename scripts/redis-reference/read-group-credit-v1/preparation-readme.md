# Read-group-credit matched preparation

Frozen for root review at clean candidate `57ff6851e40ed63c837189d6eb0a11190704725a`. `READINESS.json` contains final source/build/helper hashes and exact argument arrays. The full release has 594 matching source files, an empty feature array and opt-level 3. Fixed measurement clients remain native `03c1` and Redis `b8ec`; the candidate release workload is not substituted into timing.

`before-final-pins/`, `PREPARATION.json`, provisional protocol and initial contract logs preserve the unfinished preparation. The final six helpers are exact pin/path/protocol derivatives of the watchdog predecessor; `final-preparation.diff` records every executable difference. The isolation wrapper, driver arguments and six-test driver contract file remain byte-identical. All six driver and ten auditor pure contracts passed with final pins; no runtime was launched here. The separate process runner is unchanged.

The design remains 24 timed cohorts (12 correctness-smoke cells): six arms, c1/c64, two reversed repeats, 5-second read-only closed-loop windows, 4096 keys, 128-byte values, 128 warmup calls, 10 M cap, 1500 ms deadlines. Compare against accepted `5ee`, not against the watchdog experiment. Preserve all original outcome/histogram/CPU/resource/drain/retention/lifetime/restoration checks. This is a shared-host volatile tmpfs-WAL three-voter diagnostic against standalone memory Redis, not equal durability or sustained capacity. Existing fixture admission/configuration bounds remain unchanged; the candidate's pending-ReadIndex credit check is the production behavior being screened.

Root correctness smoke command:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 python3 /tmp/kv9-read-group-credit-comparison-preparation/matched-driver.py --client-source /tmp/kv9-point-batch1-measurement --client-build /tmp/kv9-point-batch1-client-release-first --expected-client-revision 03c1c776a5dd7d1cc67491ab253e02ce51665bf8 --output /tmp/kv9-read-group-credit-comparison-smoke-first --smoke
```

Root timing command, after smoke and quiet-window clearance:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 python3 /tmp/kv9-read-group-credit-comparison-preparation/isolate-and-run.py --driver /tmp/kv9-read-group-credit-comparison-preparation/matched-driver.py --driver-arguments-file /tmp/kv9-read-group-credit-comparison-preparation/driver-arguments.json --output /tmp/kv9-read-group-credit-matched-diagnostic-attempt1
```

Read-only audit command after the exact timing session is terminal exit 0; replace only the `ROOT_TERMINAL_TIMING_SESSION` argument with that observed session ID:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 python3 /tmp/kv9-read-group-credit-comparison-preparation/audit.py --run /tmp/kv9-read-group-credit-matched-diagnostic-attempt1/cohorts --output /tmp/kv9-read-group-credit-comparison-preparation/results-first --expected-driver-sha256 68c1bd5a78f4a24c4e05cb32dc080b8e87165ad04b2453b97b2be08e984290cc --root-session ROOT_TERMINAL_TIMING_SESSION --root-exit-code 0 --successful-arms 24
```

All outputs must be fresh; preserve any original failure. Final hashes are in `READINESS.json`; `frozen-inputs.json` indexes the completed preparation and preserved first attempt. No additional permission mechanism was added to the driver.
