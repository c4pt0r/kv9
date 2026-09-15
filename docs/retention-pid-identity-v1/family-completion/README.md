# Actual coverage of all four corrected retention readers

All four PID-identity-corrected reader families now have a completed original
cohort, full original-object readback and final COLD state. The
[combined report](family-completion.json) binds their actual evidence.

| Corrected reader | Original plan ordinal | Successful current readback decoders |
| --- | ---: | ---: |
| FNV writer | 013 | 344 |
| Full CRC regression | 014 | 303 |
| Published directory | 020 | 150 |
| Frame-buffer CRC | 021 | 150 |
| Total across these four accepted runs | | 947 |

The new independent audit checks only directory020 and frame021 (`bb2219/0`):
original selection/catalog scope, actual restoration, full corrected readback,
final COLD, controller release/completion and phase receipts. All 300 newly
checked decoder lifetimes have successful exits and are gone under the recorded
PID, start time and boot ID. The combined report (`289851/0`) reuses the accepted
FNV013 and CRC014 evidence by exact hashes. It does not replay their payloads or
lifetime checks. The 1,894 inherited historical producer counters remain
separate from the 947 current readback decoder receipts.

The [exact evidence archive](independent-evidence.tar.gz) preserves ten files,
221,887 original bytes, including audit sources, detailed observations, tool
terminals and the original inventory. Full gzip EOF and every member's complete
bytes pass publication readback (`952d91/0`). The combined report and tool
terminals are exact copies; the readable inventory and allocation files are
JSON formatting views of the originals preserved in the archive.

The original report allocates 245,760 bytes, separately recorded outside the
running continuation's fixed nine-root accounting. Publication adds separate
reporting costs; neither is claimed as reclaimed space. The frozen running
controller and its accounting are unchanged.

This completes corrected-reader representation coverage. The capacity campaign
still requires its actual terminal result and fresh capacity/exclusivity checks
before the unchanged eight smoke and sixteen timed receipt-tail cohorts. It
establishes no new database performance result or industrial roadmap completion.
