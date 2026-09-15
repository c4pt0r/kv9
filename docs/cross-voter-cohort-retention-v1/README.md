# Qualified whole-cohort WAL retention migration

On 2026-09-15 UTC, one completed directory-write cohort passed lossless
cross-voter migration, exact restoration and its unchanged whole-cohort reader.
The final transaction recovers **924,991,488 allocated bytes** after charging
all transaction files, including patches, toolchain, scratch directories and
receipts. Portable publication files are additional reporting overhead.

| Actual scope | Result |
| --- | ---: |
| Original compressed objects in the complete cohort | 150 |
| Original logical bytes fully read by the original reader | 4,015,670,348 |
| Original compressed bytes validated by that reader | 2,838,288,732 |
| Eligible n2/n3 engine segments replaced by n1-based patches | 80 |
| Patch bytes, excluding already retained n1 bases | 1,397,799 |
| Selected original allocated bytes | 946,053,120 |
| Final transaction allocated bytes | 21,061,632 |
| Local controls passed | 19 |
| Actual codec lifetimes, all successful and absent | 870 |

The selected cohort is directory timing `003-new-batch64-r000-p00001`.
All 70 other objects, including every n1 base and non-engine Raft object,
remain present with their original complete hashes. Original catalogs,
benchmark reports, resource samples and acceptance records remain unchanged.
The 1.4 MB patch total is not a base-inclusive compression ratio.

## Executed sequence

1. Run 18 transaction/accounting controls and one actual Linux parent-death
   control. Cases include interrupted rename/unlink, changed input, destination
   collision, missing authority and conservative interrupted-decode accounting.
2. Stage all 80 patches without removing originals; retain the exact encoder,
   dynamic loader and four libraries.
3. Independently reconstruct every original WAL and its exact former compressed
   bytes. This phase actually executes the retained toolchain and verifies both
   original SHA-256 identities and lengths.
4. Explicitly release the first retirement using the completed verification hash.
5. Restore all 80 compressed objects to their original paths without overwriting
   collisions. File bytes, mode, owner and mtime are restored; inode/ctime are new.
6. Run the original whole-cohort reader unchanged over all 150 objects. Its
   original metadata, history, resource, child and complete-decoding predicates
   all pass. Its historical `codec_lifetimes` field is not a new process count:
   the fresh reader executes 150 decoders, following 720 migration codecs.
7. Separately release retirement cycle 2 using the actual verification,
   restoration and original-reader result hashes. The final state is COLD.

The original reader requires restoring the compressed object paths before a
future read. The prepared bounded `restore_2` and `unchanged_legacy_2` commands
remain available; neither was executed in this qualification. The current COLD
representation is not transparent compatibility with the original reader.

The prospective migration policy reserves 12 GiB plus metadata at launch,
maintains 8 GiB available, caps each codec at 512 MiB address space, limits
migration outputs to 128 MiB, and charges up to 4 GiB including external restored
targets. The unchanged original reader retains its larger original member bound.
Cumulative decoding is charged across phases and interruptions; this execution
charges 12,036,960,356 bytes against 32 GiB. These are local operational policies,
not database consistency or durability changes.

The root final reporting attempt initially applied the 128 MiB migration-output
hash helper to a larger preserved Raft object and refused. That reporting failure
is retained. The corrected independent audit streams each preserved object under
its original size bound; no migration or workload rerun was needed. Source fsync
ordering and synthetic interruption controls do not establish power-loss testing.

## Evidence and next action

`original-evidence.tar.gz` retains 3,759 original source/reporting files totaling
18,815,272 bytes, including all phase receipts, review findings, the first audit
failure and the corrected final audit. Run `python3 docs/cross-voter-cohort-retention-v1/verify.py`
to read every archived byte and validate phase bindings, scope and accounting.
This performs no extraction, codec execution or database rerun. Original base
objects, patch payloads and retained executable payloads remain local; this
reporting archive alone cannot restore the WAL objects.

The local transaction is
`/tmp/kv9-cross-voter-directory-cohort-003-transaction-20260915-first`;
its frozen commands are under
`/tmp/kv9-cross-voter-cohort-migration-preparation-20260915-first/ROOT-COMMANDS.json`.
The successful root phase records are under
`/tmp/kv9-cross-voter-cohort-migration-root-20260915-first`.

Available space at the final audit was 25,352,613,888 bytes, before portable
publication overhead. The receipt write comparison still needs an estimated
80–100 GB available. Extend migration in bounded groups and stop on actual free
space; no additional recovery or new database QPS is claimed here. Older campaign
readers need a separately reviewed live readback-capacity policy adaptation;
their historical validation and original acceptance must remain intact.

All work is local. This stage closes no original industrial roadmap checkbox.
