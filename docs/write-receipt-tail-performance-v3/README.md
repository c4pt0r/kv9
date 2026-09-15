# Receipt-tail matched write screen: original evidence

The unchanged **8-smoke / 16-timed** write screen completed successfully for
receipt-tail `a6ac335ef4567f2e1a2b33da1a723d694cd5b7d9` versus selected CRC
`bd42e60f84657e22e36e34924a5c80a08eac623a`, using the fixed native-v3 client
`0be806d9671e2c50701a64aa7889c8859b7648ba`. Point Put and BatchPut64 ran at c1/c64,
with 2-second smokes and two opposite orders of 10-second measured cohorts.
The original storage-v3 resource, retention, readback and lifecycle predicates
remain in the archived protocol and helpers.

| Actual stage | Tool session / terminal |
| --- | --- |
| Eight smokes | 18934 / 899209 / exit0 |
| Smoke accounting readback | direct d56257 / exit0 |
| Sixteen timed cohorts | 84770 / 38c0ab / exit0 |
| Independent final audit | 16800 / 8faaea / exit0 |

The [original audit acceptance](audit-acceptance.json) records **7,296,894 timed
calls / 64,701,927 input items**, all one-attempt successes, with zero failures,
unknown outcomes or dropped slots. It binds 64 exited timed lifetimes, 48 fresh
drains and 48 voter/writer/listener bindings. Original independent retention
readback covered **73,743,331,446 logical bytes** across smoke and timing.
Those are retained execution results, not new checks performed by publication.

The [initial preflight failure](initial-preflight-terminal.json), **bb46f2/1**,
is preserved separately: the reporting reader expected `rows` where the frozen
inventory used `files`. It failed before workload launch. Successful runtime
receipts do not replace or relabel that failure.

## Exact portable metadata

[evidence.tar.gz](evidence.tar.gz) contains **169 original files / 35,047,431
bytes**, plus its inventory. It includes the frozen preparation/readiness
scripts, protocol, commands, source deltas and inventories; actual preflight,
launch and terminal records; all five final audit JSON outputs; both matrices;
timing isolation/restoration metadata; source/build role manifests; and all
**60 original inputs** bound by the separate calculation, including the 16
report/configuration/resource-coverage triples and unchanged histogram helper.

The runtime archive is **2,069,166 bytes**, SHA-256
`cf0f0eae5d3d9d89eb888487b58e4a01ab6ea6baadf557c36499482b034e8e32`.
[members.json](members.json) preserves exact original paths, sizes and hashes.
Packaging passed **13101d/0**; [independent readback](verification.json) passed
**f8e95d/0**, reading the complete gzip stream through EOF and comparing all
**170 members** once.

WAL/compressed payload objects, executable bytes, full source worktrees and
GitHub snapshots are excluded. The included build/source, logical-original,
physical-retention and audit-input inventories retain their identities. This
bundle supports review of the reported pooling inputs and original acceptance;
it does not provide an offline replay of source, process, filesystem or full
WAL acceptance. Historical preparation fields such as `runtime_ready=false`
remain exact; later actual execution receipts carry the completed acceptance.

## Separate frozen analysis

[summary.json](summary.json), [input-hashes.json](input-hashes.json) and
[ANALYSIS.md](ANALYSIS.md) are exact copies of the separately produced analysis.
Extraction passed **8970e2/0**, with freeze **022040/0**. Publication did not run
the calculation again or change its table. The decision remains **keep CRC,
hold receipt-tail**; the top-level performance report owns that assessment.

[analysis.tar.gz](analysis.tar.gz) preserves all eight original analysis files
and their original inventory, plus its publication inventory: **10 members**,
**22,287 compressed bytes**, SHA-256
`109f2905cc84e40dd5b26e34432e93f51bc3ee109d02bf913aeffe7e22943e74`.
It includes the original extraction source, terminal/stdout/stderr and retained
initial inspection note. Packaging passed **771a0b/0**; its single complete
[gzip/member readback](analysis-verification.json) passed **d65608/0**.
The runtime archive was preserved unchanged when this bundle was added.

## Reporting commands and limits

The recorded commands use helper CPUs6–15,22–31. `package.py` and
`package-analysis.py` require the original metadata paths and absent archive
destinations. `verify.py` and `verify-analysis.py` need only their corresponding
archive/member inventory and an absent verification-output destination.

```sh
sudo -n env PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 taskset -c 6-15,22-31 python3 package.py
env PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 taskset -c 6-15,22-31 python3 verify.py
sudo -n env PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 taskset -c 6-15,22-31 python3 package-analysis.py
env PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 taskset -c 6-15,22-31 python3 verify-analysis.py
```

The finite runtime selection is capped at 64MiB original metadata and 8MiB per
member; the analysis selection is capped at 1MiB. Each verifier bounds compressed
and decoded input and refuses extra, missing, duplicate or changed members.
[reporting-terminals.json](reporting-terminals.json) retains actual tool results;
[inventory.json](inventory.json) pins the publication files. Reporting allocation
is separate from historical benchmark retention/resource accounting. No build,
test suite, benchmark or payload codec was run for this publication.
