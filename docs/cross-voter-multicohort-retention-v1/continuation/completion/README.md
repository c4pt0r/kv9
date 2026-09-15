# Continuation completed at the actual capacity target

Actual session **85630** ended **b96c6f/0**. Root's terminal metadata validation
passed **a18ff2/0**. The [completion record](completion.json) binds the successful
child terminal, final status and controller result.

| Final boundary | Recorded value |
| --- | ---: |
| COLD plan cohorts | 40, ordinals000–039 |
| New continuation cohorts | 26, ordinals014–039 |
| Raw `completed_ordinals` | 39, excluding bootstrap000 |
| Available bytes | 85,304,766,464 |
| Conservative net recovery | 59,995,123,712 bytes |
| Prospective continuation stop | 85,000,000,000 available bytes, reached |
| Historical plan target | 100,000,000,000 available bytes, not reached |

The accepted prefix000–013 remains separately bound. Each new completed record
binds its selected scope, verified release, exact restoration, complete corrected
whole-cohort readback and final COLD result. Detailed source/phase results and
all retained data remain their original local authorities. This publication
copies reporting records; it does not replay payload checks or establish a new
benchmark result. Later benchmark readiness uses its own fresh preflight.

The older campaign and its ordinal013 reader failure remain **FAILED**, unchanged.
The [reporting correction](reporting-correction.json) also preserves the later
reporting-only assertion failure **9fbfc4/1**: raw39 was initially compared with
40 total plan cohorts. Bootstrap000 explains the difference. No runtime
failure, source change or payload replay resulted from that reporting error.

## Portable original records

[terminal-metadata.tar.gz](terminal-metadata.tar.gz) contains **267 exact original
JSON files / 714,436 bytes**, plus its member inventory: terminal reports, startup
binding, and the complete/release/stage-verify/finish records for014–039. It omits
WAL objects, tools, large phase receipts and unrelated publication/GitHub files.
Their original paths and hashes remain in the included authoritative records.

Archive SHA-256:
`5ed1fe52ef1ff7794dbc7944e1dd113a347724b5c7a2804c7cb956b96ae943eb`.
The archive is **65,095 bytes**. [members.json](members.json) maps every member
to its original path, byte count and hash. [verify.py](verify.py) independently
read the complete gzip stream through EOF and checked all **268 members**
against that inventory: **75149a/0**, after packaging **074d75/0**.
[verification.json](verification.json) records the exact results; no data codec
or workload was invoked.

The finite original-path reconstruction command is:

```sh
sudo -n env PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 taskset -c 6-15,22-31 python3 publish.py
env PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 taskset -c 6-15,22-31 python3 verify.py
```

`publish.py` requires an absent archive destination and the retained original
paths; it is a reporting exporter, not a retention restore command. For portable
readback alone, run `verify.py` beside the archive and member inventory with an
absent `verification.json` destination. Existing published files are preserved.

Final controller accounting predates this publication. Its nine extra roots do
not include this new completion directory. [reporting-allocation.json](reporting-allocation.json)
records this directory's allocated bytes and the parent README allocation delta
separately; neither is retroactively counted as controller overhead or additional
recovery. [reporting-terminals.json](reporting-terminals.json) retains the actual
reporting tool receipts and source pins.
