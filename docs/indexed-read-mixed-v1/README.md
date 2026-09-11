# Indexed-receipt read/mixed evidence

This compact selection contains 192 exact files: 39,543,246 decoded bytes and
1,062,983 compressed bytes. `inventory.json` binds original paths, member hashes
and archive-part hashes. All 24 original reports and resource-coverage records
are included, along with preparation, original build records, accepted audit,
statistics and authoritative terminal receipts.

Raw WALs, runtime binaries, full environment state and unrelated host process
listings remain local. This is not a complete runtime archive. Earlier proof
and recovery evidence stays linked from the [report](../INDEXED-RECEIPT-READ-MIXED.md).

```sh
python3 docs/indexed-read-mixed-v1/verify.py
```

The verifier performs byte-integrity checks only. It does not run a benchmark,
audit, proof or fault campaign. [READOUT.md](READOUT.md) and
[PER-REPEAT.md](PER-REPEAT.md) copy the original statistical output unchanged.
