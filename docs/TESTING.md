# Testing contract

Every rule here exists because it failed on this project on 2026-08-28, during Phase 1 and
the M1 raw/membership work. The failure is recorded with the rule. **A rule with no visible
origin gets overturned on principle by the next person who finds it inconvenient** — so if
you are about to relax one of these, read what it cost first.

Contributor is named per rule, because the rule and the person who paid for it are different
information: the name tells you who to ask, the failure tells you why arguing with it is
expensive.

---

## 1. One mutation, one defect

A mutation used to prove a test discriminates must introduce **exactly one** defect.

*Origin (Cindy):* to prove the raw E2E detects a bypassed Raft write, I mutated
`commit_batch` to write the local engine directly **and** to return a hardcoded
`AppliedPosition{1,1}`. Two defects. My red landed on the position check; Ren's
single-defect version of the same mutation landed on the cross-failover read-back. I read
the difference as "replication is guarded by two independent assertions" — wrong. They were
two *defects* each caught once.

*Correction (Ren):* the two guards are not equivalent and must not be recorded as such.

    cross-failover read-back : LOAD-BEARING. To go green the data must genuinely reach
                               another node. No bypass-Raft mutation can pass it.
    (term,index) position    : TRIPWIRE. Guards "did the reported position lie" —
                               correlated with replication, not the same property.
                               A mutant returning the real position walks straight past it.

Why the distinction earns its keep: recorded as "two independent guards", someone deleting
the cross-failover read-back sees the position check still standing and concludes strength is
unchanged — having removed the load-bearing wall. Recorded as "one load-bearing, one
tripwire", **what each deletion costs is computable**.

## 2. Prove the red before you trust the green

A test is evidence only after you have seen it fail for the reason you intend it to catch.
Delete or invert the thing under test; require the specific test to go red.

*Origin (compile-time `'static` assertion in `MemEngine` — idea Cindy, implementation and the
experiment below Ren; see the credit in `3a30a7a` and the code comment):* the assertion
appeared to work, but the first reverse test produced two compile errors and the unrelated one
would have fired regardless. Redone with the refactor completed properly, the assertion was
the *only* remaining error, and the full suite stayed green without it. Until that second
attempt, "the assertion is load-bearing" was belief, not evidence. **A reverse world must be
clean before a red is attributable.**

*Additional case (Ren, on the `FaultyEngine` feature gate):* his first reverse test knocked on
one door only — `kv9_engine::testing::FaultyEngine` — while the type was equally reachable via
the root re-export `kv9_engine::FaultyEngine`. Gate the `mod` but miss the `pub use` and that
test still goes red, reporting a closed door while the other stands open. A correctly designed
control producing a false green.

## 3. A red must be explained, not merely counted

`exit != 0` is not the finding. **Where** it went red is the finding.

*Origin (Cindy):* I ran the raw E2E against a pre-implementation head to prove it
discriminates. It failed — at `error: client command must be create-keyspace`, three layers
before the code under test, with `Unimplemented` appearing zero times in the log. The run
never reached what it was meant to test. Reporting on the exit code alone would have
certified a control that proved nothing.

Corollary: a red landing outside the code under test is **not discriminating**, and must be
reported as such rather than as a pass.

## 4. A green must state what it did not cover

*Origin (Cindy):* my 5/5 acceptance run on the pair-defect fix proved no regression — and
nothing about the defect, because the acceptance script contains **zero** membership
operations, so an unfixed build passes it identically. The proof lived in someone else's
mutation, and saying "5/5 PASS, hold lifted" without that distinction would have manufactured
exactly the false confidence the hold existed to prevent.

## 5. Gates may be progress-shaped; conclusions must be exact

Waiting for "the value moved" latches on intermediate states — a leadership change is a
sequence, not an instant, so "it advanced" is satisfied halfway through and the script then
asserts against a real-but-premature snapshot.

*Origin (Ren).* Rule: **progress-as-gate is fine; progress-as-conclusion is the trap.**

Load-bearing conclusions must compare exactly (`-eq`, exact `(term,index)`, exact expected
voter/learner lists) **and fail hard** — a bare `test` under `set -euo pipefail`, not a
polling predicate.

*Extension (Ren, verifying `phase1-final-acceptance.sh:219-220`):* an early-latching gate is
tolerable only when **the load-bearing conclusion behind it fails hard, and that failure
direction cannot be inverted by `set -e` semantics or a pipeline**. Under those conditions
skew can produce a false RED but never a false GREEN. Verify the failure direction, not only
the gate's tightness.

## 6. Success markers must be explicit and exclusive

Each script has its **own unique, exact completion marker**. The consumer must match that
complete marker independently. The marker may only be producible by the genuine completion
path — and **the script's `exit 0` is not a substitute for it**. Never infer success from the
absence of a failure marker.

*Origin (Ren), the sensitivity experiment and the positive-marker consumption rule:* three
controls on the same harness — normal completion left `completeness OK`, a mid-run `SIGTERM`
left `INCOMPLETE`, a mid-run `SIGKILL` left **nothing**, output simply stopping. `trap ... EXIT`
does not survive `SIGKILL`, so a killed harness is indistinguishable from a finished one.
Hence: judge a batch complete because you *saw* the success line, never because you did not
see a failure line. Absence of a failure marker has two causes.

*Adopted into team acceptance (Cindy)*, along with the accompanying admission: I had proposed
the `trap` as the fix and never tested the detector itself.

*Exclusivity — rule by Tess, failure by Cindy:* a generic `grep -q '^PASS'` is satisfied by any
line starting with PASS. Use the script's own unique marker with `grep -Fq --`. Cindy had
confirmed all three scripts print PASS at line start and stopped there; Tess asked the other
half — whether anything *else* could match. It is the same "this path is closed ≠ all paths
are closed" error Cindy had named an hour earlier about a `pub use` re-export.

Beware the fix: `run: ./script.sh | tee out` hands the step `tee`'s exit code. Use
`set -o pipefail`, and keep the assertion a separate step so "script failed" and "script
exited 0 without reaching its marker" stay distinguishable.

## 7. Filtered verification: pre-declare N

`cargo test <filter>` matching **zero** tests exits **0**:

    test result: ok. 0 passed; 0 failed; 7 filtered out      exit=0

*Origin (Ren):* while verifying a leak guard, his filter matched nothing and reported green.
It was caught only because he had written `want non-zero` before running.

*Recipe (Tess):*

    1. declare the expected selected count N *before* running the filter
    2. baseline, unmutated, same filter: exactly N selected, all passing
    3. single-defect mutant, same filter: exactly N selected, failing at the
       expected assertion
    4. restore, same filter: exactly N selected, all passing

Step 2 is the one that catches a **half-empty** filter — one that runs 1 test where it should
run 9. Without it, "no red" and "the filter selected nothing" are indistinguishable, and that
ambiguity lands precisely on the mutation round where it does the most damage.

*Note:* a personal wrapper is not this contract. A criterion everyone must satisfy cannot
live on one person's PATH (Cindy) — the recipe belongs here, and `cargo-mutants` in CI is the
mechanical endpoint that removes hand-written filters entirely.

## 8. "The assertion exists" ≠ "the endpoint executed it"

A pure-predicate unit test proves only that the rule **can be expressed**. Proving that the
real entry point consults it requires separate **traversal-sensitive** evidence: production
and tests calling the *same* function, exercised through the production path. A gate verified
via a parallel implementation is not verified.

*Origin (Tess, reviewing the region/epoch gate):* the pure range predicate was tested; nothing
showed any real endpoint reached it.

*Ren's summary:* extracting the rule into a pure function made its asymmetry readable, then
he counted that readability as coverage — "implementation exists" passed off as "it is called".

## 9. Public error mapping stays exhaustive; no wildcard

`error_status`'s match has no `_ =>` arm, so a new `Error` variant fails to compile until it
is explicitly mapped.

*Origin (Tess's charter; demonstrated when Ren added `ObjectContentMismatch` and hit
`E0004`).* Its protective power comes specifically from **the absence of a wildcard arm in an
exhaustive match** — not from absences in general, which have no inherent protective value.
That is also its fragility: adding `_ => Status::internal(message)` lets every future variant
compile, keeps all tests green, and silently removes the mechanical mapping gate. It looks
like tidying up.

Wire text must not carry object keys or physical paths.

## 10. Write down what a claim is *conditional* on

Mapping choices and safety arguments rest on conditions that later work changes.

*Origin (Cindy, on `ObjectContentMismatch → Status::internal`):* the stated reason was
"clients cannot reach it — object writes come from the drain worker". True conclusion, wrong
reason: no drain worker exists, so it is unreachable because **nothing** calls it. Record the
condition the mapping depends on, that it is currently vacuous, and the moment it must be
re-checked.

## 11. No existence claim without a ref

*Origin (Cindy):* I grepped the working tree (master @ `a75443a`) and reported "no `NotLeader`
anywhere in the repo". It already existed on another head. With several worktrees in play,
"does the repo contain X" has no answer.

    git grep <pat> <sha>              answers "at this commit"
    git log --all -S'<pat>'           answers "on any branch"

State which. Related: **an unauthorised negative is not evidence** — querying branch
protection unauthenticated returned `401`, which cannot distinguish "no protection" from "not
allowed to look". `404` from an authorised query is the answer; `401` is not.

## 12. Verify the tool you verify with

*Origin (Ren):* his wrapper enforcing rule 7 was itself checked four ways — no match fails,
correct filter passes, `min` above actual fails (not merely `>0`), and test-ran-but-failed
exits distinctly. His first attempt at the fourth was void: he reused a non-matching name and
merely re-ran the first case.

*Extension (case Cindy, rule as worded by Tess):* independent verification items must not be
joined by an `&&` chain that silently skips the remainder — unless the short-circuit is itself
the declared and verified semantics. Each item must visibly report executed / not-run and its
exit code. `;` separators with per-item `rc` logging is the recommended implementation, not a
blanket prohibition on `&&`.

The case: checking whether a type was a pure-move candidate, my first grep found nothing and
exited 1, so the remaining three checks never ran. Their absent output is indistinguishable
from "ran, found nothing" — one step from reporting no references in files that contain six.
Rule 6's shape in a new carrier: not *no failure marker seen*, but **no output seen** taken as
*check completed and empty*.

*Corollary (Cindy):* never pre-write the interpretation into the command. `echo "(blank =
clean)"` appended to a check is a prediction wearing the costume of a result — twice mine
printed alongside output that contradicted it. Print raw output, read it, **then** conclude.
The disciplined version of the same instinct is rule 7's pre-declared N: a falsifiable
prediction rather than an asserted outcome.

## 13. Quantified evidence must come from a command, not from memory

*Rule as worded by Tess:* a **quantifiable fact used in testing, acceptance, or a release
gate** must be produced by a reproducible command before it enters a conclusion, and enough of
the raw result must be kept to re-check it. Do not copy a number from memory, and do not infer
a stable behavioural tendency from the direction of a few errors.

"Enough to re-check" means the exact command, the target head, the exit code and the count —
not an entire log pasted into a document.

Rule 12 governs whether every check actually ran. Rule 13 governs whether what they produced
is carried faithfully into the conclusion. Verifying carefully and then reporting a remembered
number spends the verification and delivers the guess.

*Origin (Cindy, three times in one day, all to the project owner):*

    described the release as a CI/workflow-level change      understated   IN SCOPE
      — it was `git rev-list --count a75443a..02d9a9b` = 40
        commits, a full day's work
    said "41 commits" where rev-list says 40                 overstated    IN SCOPE
      — written in the message correcting the one above,
        from a list printed and never counted
    said "44 NotImplemented stubs" of work that left the     overstated    out of scope —
    count at 44 before and after, so it read as *cleared*    completion    progress report,
                                                                           not test/acceptance
                                                                           /release evidence

The last is kept because it is where the habit shows, but it sits outside what this rule
governs. Two of the three land inside it.

Two of these understated and one overstated, and on the strength of the two I told the owner I
bias toward smaller/simpler — a theory the third killed within the hour. **The constant is not
a direction, it is a missing action** — and a wrong self-diagnosis is worse than none, because
guarding against understatement catches nothing when you overstate.

*Second origin for the same half (Ren):* he had already written that false generalization into
his own notes as a settled conclusion, on the strength of those same two data points. The third
falsified it. He marked the section FALSIFIED in place and wrote the correction rather than
deleting it silently, which leaves the next reader able to see what happened.

*Propagation requirement (Tess):* **if a conclusion has been copied into several records, the
correction that follows a counter-example must be propagated to every known copy; an
uncorrected copy must not be left to pose later as independent corroboration.**

Here the false conclusion had landed in **two** separate notes files and both of us removed it
independently. Had only one of us corrected, the surviving copy would later have read as a
second source — two records that were one unchecked idea.
