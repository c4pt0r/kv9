# Vectored WAL write performance evidence

The complete 8-smoke/16-timed comparison passed independent acceptance.
All 6,961,558 measured calls succeeded once, with no errors, unknowns or drops.
The isolated candidate is not promoted: loaded point throughput changes
-0.612% and BatchPut(64) throughput -1.020%, with worse pooled p99 intervals.

Read [pooled results](REPORT.md), [both repetitions](PER-REPEAT.md),
[directional comparisons](COMPARISONS.md) and [complete summary](summary.json).
Selected server is `11113f6`, vectored candidate `cfd9c92`, fixed native v3
client `0be806d`. Both use three-voter volatile tmpfs WAL on one shared host,
with unchanged Raft, sync and response fences. This is not a real-disk,
cross-host, sustained-capacity or default-promotion result.

The 19 archive parts preserve 2,351 reporting/configuration/resource/lifecycle,
source-binding, readback and retention-metadata files (315,866,547 decoded bytes;
38,252,019 compressed bytes). Original startup wrapper refusals, successful
runtime receipts and arithmetic checks remain. No failed cohort was rerun.

Large WAL objects and executables remain local, with their original manifest
and catalog hashes in [omitted payload references](omitted-payload-references.json).
The actual campaign audit independently decoded all 64,532,128,628 WAL bytes.
This archive's byte verification does not rerun that acceptance or establish
current availability of external local payloads. The original reporter is
included; its source paths describe the original host layout.

Verify all portable bytes without extracting or executing the contents:

```sh
python3 verify.py --inventory-sha256 b85c6afffffce7a4afa8eb57b3ba2155d4dcb84d0510efa3cb2f0b54bd3c6912
```

[Publication receipts](publication-readback.json) retain the actual successful
packaging and independent readback. Timing was root session 95283, terminal
4c9a5f/0; full audit 16167, e16a89/0; summary 516849/0. No hosted CI was used.
