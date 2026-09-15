# Late negative settlement proof coverage

The checkpoint-owner model permits a rejected submission after an earlier
submission of the same identity applied. Production Pending retention is
unchanged. Seven theorems / 43 strict obligations, both finite models, six
fault counterexamples and three proof-rejection controls pass. An additional
constrained reachability trace applies the effect before observing negative
evidence, then clears the journal with Pending still Published and no Version.

`evidence.tar.gz` contains both original runs, exact model/proof/configuration
inputs, result records, all logs, the corrected runner and its original source.
The first run stopped at an ambiguous mutation selector; it is not a passing
run. The current runner selects `CApply` explicitly. The JSON manifest binds
every payload. `readback.json` records a complete byte-for-byte archive readback.
No runtime behavior, deletion safety, certified abort release or end-to-end
liveness is added by this model correction.
