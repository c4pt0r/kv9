# Indexed quorum read receipts

Tracking: #13, #20 and #9. This candidate replaces the driver read-confirmation
vector scan with a bounded FIFO and a tree index by exact context bytes. It does
not change ReadIndex submission, the completion notification protocol, local
apply catch-up, authorized snapshot acquisition or request deadlines.

## Observable contract

The reference representation is a vector `Q` of `(context, index)` pairs, keeping
the last `N` inserted pairs. Lookup returns the index of the first retained pair
whose entire context equals the query. A lookup neither consumes the receipt nor
makes it more recent. Duplicate contexts may have different indexes; their first
retained occurrence wins. This contract deliberately does not assume unique
contexts, increasing indexes or a particular context encoding.

`ReadReceipts` stores a FIFO of contexts and a `BTreeMap` from each context to its
FIFO of retained indexes. The driver still holds one leaf mutex while publishing
a Ready's receipts, and while looking up a confirmation. The capacity stays
1,024. An evicted or never-confirmed context still cannot produce a hit.

For `K` distinct retained contexts, lookup uses `O(log K)` context comparisons
instead of scanning up to `N` pairs. Insertion and eviction have tree operations
and amortized constant-time FIFO operations. Context comparisons depend on byte
length. There are at most `N` FIFO context entries and `N` retained indexes across
the tree, with at most `N` nonempty tree entries. The tree owns an additional copy
of each distinct context. This is an explicit allocation/memory tradeoff; it is
not a complete process-byte bound or a guaranteed speedup.

## Representation proof

For any finite capacity `N >= 0`, maintain these relations to the reference `Q`:

1. The context FIFO equals the contexts of `Q`, in order, and `|Q| <= N`.
2. For every context `c`, the tree's index FIFO equals the indexes of all pairs
   in `Q` with context `c`, in their original order.
3. A tree key exists exactly when that filtered sequence is nonempty.

The empty constructor establishes all three relations. At capacity zero,
insertion leaves both representations empty. Otherwise consider an insertion:

- If the FIFO is full, the reference drops its first pair `(c, i)`. The context
  FIFO removes exactly `c`. By relation 2, `i` is the first index in the tree
  FIFO for `c`; removing that index preserves the filtered order of all remaining
  pairs. Removing an empty tree FIFO preserves relation 3. Other contexts are
  unchanged.
- Append the new pair `(d, j)` to the reference. Appending `d` to the context FIFO
  and `j` to the tree FIFO for `d` establishes relations 1 and 2 for the new
  sequence. Creating a missing tree entry preserves relation 3. This also covers
  `c = d`, repeated indexes, and duplicate contexts with different indexes.

Induction therefore preserves the relations for every finite insertion history.
By relations 2 and 3, a missing tree key is exactly a missing reference match,
and a present tree FIFO's front is exactly the reference's first retained match.
Repeated lookup changes neither representation. Batch publication is equivalent
because repeatedly retaining a suffix of length `N` while appending yields the
same final suffix as appending the full batch and retaining once. The existing
publication mutex hides intermediate insertion states from readers.

This is a mathematical representation argument tied to the implementation;
it is not a mechanically verified Rust theorem. The indexed container stores
already-confirmed receipts. Quorum authority, context lifetime, completion races
and apply/snapshot ordering still rely on their existing protocol contracts and
separate proof/refinement evidence.

## Validation and acceptance

Focused tests cover exact bytes across incarnations and prefixes, nonconsuming
lookup, FIFO rather than LRU eviction, duplicate contexts, and first-match
behavior after repeated eviction. An independent vector specification compares
every lookup after every prefix of all length-seven histories over three
contexts, at capacities zero through four. It also checks the representation's
entry bounds and absence of empty tree FIFOs. These finite executions support
the representation argument and do not replace its quantified proof.

Local validation passes 611 workspace tests and doctests, with 23 ignored
external/optional tests, and warning-denying all-target workspace Clippy.
`scripts/check-read-receipt-controls.py` accepts baseline/restored source and
rejects three compiling mutants at named assertions: latest rather than first
duplicate match, omitted FIFO eviction, and confirmation without exact context.
Every copied source file and each test transcript is hashed in the retained
control manifest. Compilation failure or zero selected tests cannot satisfy a
control. The exhaustive test checks 76,545 insertion prefixes and 306,180 lookups
against the vector specification.

Performance and process/fault validation remain pending for this candidate. Its
source was prepared in a separate worktree while exact Ready892b2a1 was being
measured; those measurements do not include this index. The compilation/test
window ran after Ready disk timing and before the separately labeled tmpfs
diagnostic. Before acceptance, complete exact-source process/partition/restart
checks and a new unchanged-protocol performance comparison. Verify the memory
tradeoff and all unsuccessful outcomes alongside throughput. Applicable exact
candidate fault and implementation-refinement gates remain open. Daily checks
remain local.
