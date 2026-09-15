# Receipt upper-bound release/recovery evidence

This packet supports [the runtime checkpoint](../WRITE-RECEIPT-UPPER-BOUND-RUNTIME.md)
for candidate `e2e23cca5e70a9ea0cc241877b3b35b5b6433d27`.

- `summary.json` records the observed release identities, actual process
  terminals, complete history totals and remaining gates.
- `inventory.json` maps every archive member to its original local path,
  byte count and SHA-256. Three excluded ELF binaries remain retained locally
  and are separately listed with their exact hashes.
- `evidence.tar.gz` contains 176 original metadata/helper/log/runtime files,
  including both complete operation histories and ordinary fixture data.
  It decodes to 5,055,583 bytes. Every member was independently read back and
  matched against the original file; all original inputs stayed unchanged.

Archive size: **694,922 bytes**. SHA-256:
`ddfa5af24cfb60dfbdc8b1903e3251b73a0408e41b4c9d9fa98128e20c280dae`.

Publication/readback completed at local tool receipt `9a43d3/0`. No successful
test or control was replayed for this packet. Actual Chaos Mesh, performance
and diagnostic capture remain separate acceptance work. Previous proof and
development failures stay in the [source checkpoint](../receipt-upper-bound-source-v1/README.md).
