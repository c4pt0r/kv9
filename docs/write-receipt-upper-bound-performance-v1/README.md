# Matched upper-bound performance evidence

Completed locally on 2026-09-15 UTC. The [performance report](../WRITE-RECEIPT-UPPER-BOUND-PERFORMANCE.md)
keeps CRC selected and holds the upper-bound candidate: loaded point throughput
improves 3.169%, but loaded BatchPut(64) loses 1.419% and its pooled p99 worsens.
Both opposite execution orders remain included. These retained releases precede
the checkpoint-owner integration on current main.

The original eight smokes and sixteen ten-second timed cohorts pass. Independent
acceptance covers 7,154,151 timed calls / 62,959,110 input items, all successful
with one attempt; 64 exited timed process lifetimes; 48 fresh applied drains;
and 48 voter/writer/listener bindings. The smoke population is separate. Full
local decoding covers 72,088,784,643 original logical bytes across both phases,
with 50,835,156,992 allocated bytes retained under `/mnt/data/kv9-work`.

## Report and provenance

- [ANALYSIS.md](ANALYSIS.md), [PER-REPEAT.md](PER-REPEAT.md) and
  [COMPARISONS.md](COMPARISONS.md) are exact copies of the generated tables.
- [summary.json](summary.json) retains integer counts, histogram intervals,
  CPU observations, source roles and all directional comparisons.
- [input-hashes.json](input-hashes.json) binds all 77 original reporter inputs.
- [members.json](members.json) lists every original archive member, historical
  source path, length and SHA-256. [copied-files.json](copied-files.json) records
  exact comparisons of the external publication copies to their local originals.

The archive contains 349 original files totaling 82,358,333 bytes, plus its
inventory: 350 tar members and 82,759,680 decoded tar bytes. This includes all
77 reporter inputs, all five final audit records, source/build identities,
original preparation and qualification failures, actual execution terminals,
the report derivation and exporter lineage.

Raw or compressed WAL payloads, executables, complete source worktrees,
credentials and issue snapshots remain outside this packet. The original audit
binds their identities, but this metadata archive cannot independently replay
full WAL acceptance. Historical absolute paths describe provenance and are not
instructions to overwrite or relocate existing files.

## Actual terminals and archive verification

| Stage | Actual terminal | Exit |
| --- | --- | ---: |
| Eight smokes | `45876 / 559f87` | 0 |
| Independent smoke readback | `905b8a` | 0 |
| Sixteen timed cohorts and restoration | `14114 / 1182dd` | 0 |
| Independent full audit | `48940 / 2f82fd` | 0 |
| Report derivation | `aad8c9` | 0 |
| Metadata packaging | `7607c0` | 0 |
| Independent archive readback | `dc1c40` | 0 |

The final audit SHA-256 is
`7f60d63edbfaa3580160b3e80bf5ed97ba01aafe4faeb8e1342a6ba82daedb4f`.
[package-result.json](package-result.json), [verification.json](verification.json)
and [publication-terminals.json](publication-terminals.json) record the completed
publication. The standalone [verifier](verify.py) checks the entire gzip stream
through EOF, exact member hashes and lengths, and zero-block tar termination
without extracting files. All checks pass.

- Archive: 4,212,385 bytes, SHA-256
  `d13c1966dcef25347e003418b9241673244ea753f217dcd8cb8d5ab65ec2a035`.
- Member inventory SHA-256:
  `c91c0c7ffa46f26a81cb30b347b8d8403f8bb5b936c85c52ddb4b0cdb99f56fc`.

Run the verifier with a fresh absolute output path on the data volume:

```sh
python3 -B docs/write-receipt-upper-bound-performance-v1/verify.py \
  --members-sha256 c91c0c7ffa46f26a81cb30b347b8d8403f8bb5b936c85c52ddb4b0cdb99f56fc \
  --archive-sha256 d13c1966dcef25347e003418b9241673244ea753f217dcd8cb8d5ab65ec2a035 \
  --output /mnt/data/kv9-work/upper-bound-publication-readback-new.json
```

[PREPARATION.md](PREPARATION.md) preserves the original pre-execution text;
its pending wording is historical. The old 64 MiB selected-original metadata
limit was found insufficient by projection, before packaging. The explicit
[reporting-only revision](reporting-cap-revision.json) raises that limit to
80 MiB. The 8 MiB member, fewer-than-512 originals, 32 MiB compressed and
80 MiB decoded-tar bounds remain unchanged. The exact finite selection and tar
envelope fit all bounds. No runtime workload, storage or codec guard changed,
and no unexecuted projection is described as a failed packaging attempt.

This is a shared-host, loopback, volatile-tmpfs write screen. It establishes no
physical-disk result, power-loss durability, current-main checkpoint performance,
Redis parity, read/mixed regression result or full-history linearizability.
The separate accepted proof, recovery and Chaos gates retain their original
scope. Hosted CI was not dispatched. No original industrial work package closes.
