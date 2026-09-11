# Sampled asynchronous read lifecycle diagnostic

This isolated diagnostic starts from the accepted two-worker runtime
`5ee897a2f58c57bdf17ea1757c96224adf0f0dbb`, retaining event interval eight.
The instrumentation and its controls are ported unchanged from diagnostic
`2ec6fcbdb339d869e2c042d62b7d7dac771eee6e`; only this source-scope paragraph
changes relative to that instrumentation patch. The old diagnostic predates
event8 and two workers, so its stage means must not be attributed to this source.
It is not a performance candidate or a production instrumentation selection.
The existing sealed-group ReadIndex, quorum/apply fences, channels, deadlines
and cancellation ownership are unchanged. Observer work can perturb scheduling
and deadlines; measurements from this build do not establish capacity gains.

A mixed hash of the process incarnation and checked request sequence samples
approximately one in 64 asynchronous registrations. It avoids selecting every
64th request, which could align with group boundaries. A sampled request and
its original ticket share one trace object; it retains only timestamps and
the node's metrics, with no strong reference to the registry or driver.
Unsampled requests carry an empty optional trace. No per-request data is exported.

The trace partitions one successfully received read into these elapsed stages:

| Metric suffix | Start | End | Included work |
|---|---|---|---|
| queue | Driver read start | Start of successful ReadIndex admission invocation | Context minting, registration, owner scheduling, deferred admissions |
| quorum | Successful invocation start | First exact-group confirmation observed | Admission callback, peer/pump work, transport, quorum progress and owner observation |
| apply | Confirmation observed | Immediately before successful-result send | Remaining pump work, applied-index coverage, completion selection and delivery preparation |
| notification | Before send | Successful receiver branch observed | Channel delivery and receiver scheduling |
| total | Driver read start | Successful receiver branch observed | Sum of the four stages above |

The five exported names use prefix `raft_async_read_profile_`. They describe
elapsed pipeline intervals, not isolated network latency or pure apply CPU.
Each member publishes an admission timestamp only when its sealed invocation
returns `Ok(true)`; deferred attempts leave no marker. The sample need not be
the group's representative. Duplicate or member-only confirmations cannot
overwrite the first exact-group marker. Confirmation is stamped separately for
each sampled member while walking the group, so it includes preceding member
notification/trace work. Send time is captured before the
channel can wake its receiver. A failed or abandoned send does not emit a
successful sample.

Relative timestamps use release/acquire atomics and a separate missing-value
encoding, preserving legitimate zero-duration stages. The receiver captures
its timestamp before recording histograms. Missing, unordered or overflowed
timestamps increment only the total metric's error population; they cannot
change the read result or admission state. Complete successful samples record
all five histograms. Existing outer read-establishment/backend timers also
include these histogram updates; their difference from profile_total must not
be interpreted as uninstrumented backend overhead. The profile has integer
sums satisfying queue + quorum + apply +
notification = total. Recording uses existing leaf observer locks outside
owner locks. It adds five fixed histograms to the diagnostic export inventory;
the existing 512-KiB bound and worst-case export test remain required.

Cross-metric exports are independently captured, so conservation is checked
only between fresh, quiescent endpoint snapshots. Counts and sums from the
same successful sample population can be compared; quantiles cannot be added.
Whole-cohort snapshots can include warmup and verification. They are not an
exact decomposition of the benchmark's separately bounded measurement interval.
Failure/cancellation traffic requires separate coverage and is not inferred
from these successful-read histograms.

Focused controls cover deferral with unchanged group membership, an unsampled representative,
exact first confirmation, apply coverage, send-before-receive, cancellation
before and after send, owner close, and arithmetic conservation. Existing Raft
tests continue to enforce the original ordering and ownership contracts.
