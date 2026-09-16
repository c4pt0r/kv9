# Packed-index experiment evidence

See [the report](../PACKED-INDEX-EXPERIMENT.md) and
[standalone source](../../scripts/resident-packed/README.md).

The packet retains 123 original source/metadata files plus the inventory:
124 members, 469,897 compressed bytes and 4,823,973 original bytes. It includes
both original failed Clippy attempts, corrected qualification/builds, all raw
samples and counters, independent analysis, the count-only probe diagnostic,
source/dependency identities and actual process terminals. Large group inputs
and executable files remain local; their exact identities are retained.

- `analysis.json`: recomputed comparisons, allocation/memory counts and failed gate.
- `analyze-original.py`: checker for the retained local originals.
- `terminals.json`: actual completed tool receipts, including original failures.
- `evidence.tar.gz`, `members.json`, `package-result.json`: complete metadata archive.
- `verification.json`, `verify.py`: successful independent bounded readback.

Verify from this directory with a fresh absolute output path:

```sh
python3 -B verify.py \
  --members-sha256 0ad56d28cc18cb747ed539f78a83b35dfe6f40da3a45a91f87b4141dcce638dc \
  --archive-sha256 915325a97e37fe912f8e76f60c247d38662b2dfde7652881b324f3957f15d0b8 \
  --output /path/to/readback.json
```

Package receipt `20ab9e/0`; complete readback `828819/0`. The verifier checks every
member hash, gzip EOF and tar termination; it does not re-execute the benchmark
or prove the tree algorithm. Original absolute paths describe the experiment
host. Production, Raft, durability and database-QPS results remain unchanged.
The fixed-stride GET-hit selection bias is retained and explicitly documented.
