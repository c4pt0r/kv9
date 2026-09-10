# Write serialization allocation candidate

Tracking: #20 and #9. Parent runtime: `11cae977f15df0912c2a35561480d45447bae660`.
This candidate reduces temporary allocations while retaining the existing
command and WAL formats, proposal protocol, and acknowledgement rules.

## Motivation and scope

The exact-parent PUT CPU recording contains recovered allocation/grow caller
frames for `Command::encode` (23 samples), `wal::encode_batch` (4), and
`RuntimeBackend::commit_batch` (6), out of 5,203 selected on-CPU samples.
These overlapping partial-stack observations motivate an experiment. They do
not attribute the much larger unspecific allocation population to these sites
or predict an end-to-end improvement. The recording is instrumented, uses
volatile tmpfs, and overlaps the separate proof gate on disjoint CPU affinities.

Changes are limited to four production files:

- `WriteBatch::into_mutations` transfers ownership of the existing ordered list.
- `Command::fenced_write_from_owned_batch` moves keys and values into the same
  fenced command shape. The borrowed constructor remains available.
- `RuntimeBackend::commit_batch` consumes its already owned batch. The command
  remains alive across the existing safely retryable replacement loop.
- Command and WAL batch encoders calculate capacity before appending the same
  bytes with the existing encoding loops.

The vector container itself may still allocate. This is not a zero-copy RPC or
engine implementation. Computing capacity adds a pass over the mutation list;
the unchanged-workload comparison must determine the net effect.

## Representation argument

Assume finite, representable input buffers, successful allocation, and the same
external protocol/storage outcomes. Allocation failure timing and wall-clock
scheduling are outside this equivalence claim.

**Owned conversion.** Let `B = [m_0, ..., m_n]`. Both constructors map each Put
to `KvOp::Put(cf_code(cf), key, value)` and each Delete to
`KvOp::Delete(cf_code(cf), key)`. Cloning and moving a byte vector have identical
byte contents. The empty lists agree. If the first `k` converted elements agree,
both iterators convert the same next mutation and append it, preserving equality
for `k + 1`. By induction the complete ordered operation lists agree, including
repeated keys and different column families. Both constructors embed that list
under `FencedInner::Write` and copy exactly the supplied region ID, configuration
epoch and version epoch into the same `Command::Fenced` envelope.

`commit_batch` still consumes its one `ValidatedFence`, returns the existing
empty-batch result before constructing a command, and proposes the resulting
command through the same `propose_and_wait` call. The command is owned by this
stack frame through every replacement retry; consuming the original batch does
not drop the proposed contents early. Fence adjudication, exact term/index
receipts, `Unconfirmed`, `FenceRejected`, and the original deadline are unchanged.

**Capacity calculation.** Define `O(ops) = 4 + sum(size(op))`, where a Put has
size `10 + key.len + value.len` and a Delete has size `6 + key.len`. The command
capacity is two version/tag bytes plus the payload length below:

| Variant | Payload length |
| --- | ---: |
| Put | `9 + key.len + value.len` |
| CatalogTxn or Write | `O(ops)` |
| ConfChange | `9` |
| Noop | `0` |
| ManifestChange | `40 + change_id.len + changeset.len` |
| Fenced Write | `25 + O(ops)` |

The WAL batch capacity is `4 + sum(size(mutation))`, with the same lengths for
Put and Delete. Each constant counts the fixed fields already emitted by its
encoder; the variable terms count the exact byte slices it appends. The empty
list and each append establish the length formula by induction. Saturating
arithmetic avoids wrapping the hint for an unrepresentable image; it does not
make such an allocation succeed. Debug assertions check emitted length against
the calculated capacity on the actual encoding path.

**Byte equality.** A new vector with reserved capacity still has length zero.
Relate old and new encoder states by equality of initialized bytes, ignoring
allocation addresses/capacities. Initially both byte sequences are empty. Every
unchanged `push`/`extend_from_slice` appends identical bytes to related states,
preserving the relation. Therefore both return the same byte string. This also
covers values whose field lengths are cast by the existing encoder; this change
does not silently add a new wire validation rule.

Equal encoded bytes preserve checksums, decoding, ordered effects and recovery
inputs. WAL frame construction, size refusal, write/sync order, poison handling,
and receipt publication are unchanged. No new queue, cached authority, timer,
singleton service, or success path is introduced. Capacity computation terminates
for each finite input and adds no wait for other requests. This argument does
not prove bounds on allocator latency or make the parent runtime's pending
protocol-composition obligations disappear.

## Validation and promotion

The compatibility regression compares both constructors over empty and mixed
ordered batches with repeated keys, all column families, different value sizes,
and distinct fence fields. It checks command equality, encoded byte equality,
decoding, and continued refusal of generic lowering before fence adjudication.
Existing workspace tests also exercise the actual encoders, malformed inputs,
epoch fencing, grouped WAL recovery, cancellation and proposal outcomes.

The full local workspace run passes 631 tests and doctests, with 23 ignored and
zero failures (`/tmp/kv9-write-serialization-workspace-first.log`). The
all-target workspace Clippy run also passes with warnings denied
(`/tmp/kv9-write-serialization-clippy-first.log`). The
exact-binary three-process fixture and performance results are recorded
separately as they complete. Performance acceptance requires a fresh parent /
candidate / parent bracket with unchanged clients and GET, PUT and mixed
workloads. tmpfs results remain explicitly volatile diagnostics. Actual
candidate-specific Chaos Mesh and the inherited checked composition gates must
be completed before runtime promotion; neither this representation argument nor
three-process SIGKILL/restart replaces those gates.
