# Testing contract

Every rule here exists because it failed on this project. Rules 1-13 come from 2026-08-28,
during Phase 1 and the M1 raw/membership work; rule 14 onward, and the extension to rule 3,
come from 2026-08-30/31, during the root-of-trust, bootstrap-barrier and Chaos work. The failure is
recorded with the rule. **A rule with no visible
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

### Extension — identify the assertion by its message, never by its line number

*Extension (Cindy):* rule 3 says **where** it went red is the finding. Line numbers are not a
durable answer to "where". One assertion in `driver.rs` was cited as `:718`, then `:729`, then
`:738` across three messages in a single afternoon; its message string never moved.

The failure this actually caused: I reported a mutation as red at "the named assertion" at
`grpc.rs:1540`. `:1540` was the `assert_eq!(status.code(), FailedPrecondition)` one line above it.
My mutation changed the status code *and* the message, so the code assertion fired first and the
named one was never evaluated — yet I cited it as proof that "two assertions each guard a class".
**No mutation I had run had ever lit the named assertion.** Isolating it required a mutation that
keeps the code and changes only the message.

*Consequence for rule 14:* when a test carries several assertions, "it went red" does not tell you
which one is guarding. Separating them requires a mutation that trips exactly one.

*Generalisation (Ren):* this governs any reference expected to outlive the tree it was written
against — assertions, cards, design docs, and **commit messages** especially, since a rebase can
never fix those. He found his own commit citing five `file.rs:NNN` for a commit scheduled to rebase
onto a head where that file is edited by three branches.

*Boundary (Cindy):* the reason is not "symbols are stable" — symbols get renamed too. Line numbers
move as a **side effect** of edits elsewhere, so nobody decided to move them and nobody had an
opportunity to fix the references; symbols move only when someone deliberately renames, and that
person can grep. The rule therefore fails wherever a symbol can be renamed without a human seeing
the references. **A message string is the strongest form: it is both symbol and content, and
changing it requires editing that exact line, so it can never move as a side effect.**

*Boundary, second half (Cindy):* "carry the value so it can be recovered" only works when the value
is distinctive. `status.role == Role::Leader` survives a rename; `check_quorum: true` and
`election_tick: 10` do not — searching for `true` or `10` recovers nothing, so those remain
name-only references wearing the appearance of content. For scalar configuration in an unfixable
reference, name the **concept** ("raft's election timeout, counted in ticks") rather than the symbol
or the value.

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

Each distorted a different quantity. Describing the release as a CI-level change understated
its **scope**; the 44-stub claim overstated **completed work** and so understated **what
remained**. Both made the thing in front of the owner look smaller, and on the strength of that
pair I told him I bias toward smaller/simpler. The `41 vs 40` **count** then overstated, within
the hour, and falsified it. **The constant is not a direction, it is a missing action** — and a
wrong self-diagnosis is worse than none, because guarding against understatement catches
nothing when you overstate.

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

## 14. Mutation controls are asymmetric: red self-proves less than you think, green proves nothing

*Origin (Cindy):* verifying that `CONNECT_BUDGET` had no regression, I removed it and both target
tests stayed green — and reported "no coverage". That green is indistinguishable from a mutation
that never applied. The conclusion happened to be right; the evidence did not support it until the
compiler was brought in as an independent witness (`warning: constant CONNECT_BUDGET is never used`
with the tests still passing).

    expected RED    self-proves the mutation LANDED — it took effect.
                    It does NOT prove WHERE: a compile error, or an unrelated broken assertion,
                    is also red.
    expected GREEN  proves nothing on its own. Needs an external oracle, and the oracle must be
                    independent of the thing whose blind spot is being measured.

*Boundary (Tess):* red does not automatically establish that the mutation landed on the path under
test. When an expected-red mutation comes out **green**, that is not yet evidence about the guard —
first exclude "the mutation did not apply" and "the mutation is not on the path this test walks".
Both happened here within one hour: @Ren's regex silently failed to match, and mine hit the
`apply_command` poison arm while the test walks the `Command::decode` arm — two visually identical
`return Err(self.poison(..))` lines.

*Corollary (Cindy):* a mutation script that fails, followed by a verification run that proceeds
anyway, prints a green that reads exactly like "the mutation applied and nothing caught it". So bind
the mutation to its confirmation — but bind only that much:

    register the restore FIRST              a `trap`, or any cleanup that runs independently of
                                            every preceding exit code — before touching the tree
    apply mutation, confirm it landed       both may fail; neither failure may reach the restore
    run the test, capture rc explicitly     an EXPECTED-RED verifier exits non-zero by design
    restore (already guaranteed), then      `git diff --quiet`
    judge the collected rcs together        the test's rc against its EXPECTED value, not against 0

*Boundary (Tess), in two parts because the first fix did not cover the second:* (a) do not chain the
restore behind the test — an expected-red run returns non-zero, so anything `&&`-ed after it never
runs, and under `set -e` the script aborts outright. (b) **`apply && confirm` has the same defect one
step earlier**: if the mutation lands and the confirmation then fails, that list is non-zero and the
shell exits before any restore that is merely "later in the script". Her minimal probe:

    zsh -c 'set -e; true && false; print RESTORED'   # rc=1, and RESTORED never prints

So the cleanup must be independent of *all* preceding exit codes, not merely placed after them.
**It is not only the test's expected non-zero that must not skip cleanup — an unexpected non-zero
from apply or confirm must not either.**

## 15. To prove an absence of coverage takes three premises, not one

*Origin (Cindy, sharpened by Ren):* "delete it and see if anything reds" is only valid under all
three:

    1. mutate to the EXTREME, not the middle
    2. confirm the mutation landed on disk
    3. confirm the test EXECUTES the mutated line

*Why 1 is not merely better evidence (Ren):* it decides whether the inference is valid at all.
Weakening a bound (2s to 2000s) leaves "still green" with two readings — no coverage, or coverage
whose threshold this weakening never crossed. Deleting leaves one. Expected-red experiments do not
need an extreme mutation to demonstrate sensitivity to the precise change they made.

*Boundary (Tess):* and the conclusion may not outrun that change. A weakening mutation going red at
the expected assertion proves the test discriminates **that one modification** — not that the whole
mechanism is load-bearing, and not that a more extreme defect would also red. Expected-red results
remain subject to the landing constraint of rule 14.

*Why 3 exists (Cindy):* a deletion-type mutation on `driver.rs`'s `apply_command` poison arm came
out all-green, and by premises 1+2 alone that is a "valid" inference of no coverage. It was wrong.

*Scope, narrower than it first reads:* a deletion makes "green therefore *the thing deleted* is not
covered" valid. It does **not** make "green therefore *this mechanism* has no coverage" valid — you
may have deleted a different entry point to the same mechanism.

*Instrument for premise 3 (Ren):* replace the target with `panic!("PROBE: reached")`. Green means the
tests never execute that line — stop and re-target. Red at that panic means the path is live; now run
the real mutation. The runtime, not your own assertion, witnesses reachability.

*Scope of that instrument, and it is not optional (Ren, on his own paragraph):* the sentence above is
unconditional and is **only true on the test thread's stack**. Cross-thread it is false in the
dangerous direction — see rule 16. Someone applying this rule to transport code will collect a green,
conclude "unreachable", and stop; transport is where this class of bug lives, and is why rule 16
exists. Read the two together, never this one alone.

## 16. A cross-thread probe's green is bounded by a window nobody declared

*Origin (Cindy and Ren, three rounds):* the probe of rule 15 does not survive a thread boundary
unchanged.

    synchronous, on the test thread's stack   panic!()           both directions self-report
    cross-thread, "was it reached?"           process::abort()   a POSITIVE hit self-proves — SIGABRT
                                                                 kills the binary, so any thread reports
    counting, or several points at once       AtomicBool/channel a written signal persists and counts

**For both cross-thread rows the negative is not self-proving**: green still means only "not observed
within W". Declare W and show it was covered before reading a green as "not executed" (Tess: do not
promise two directions in the table and withdraw one in the prose).

A `panic!` inside a spawned task does **not** fail the test: spawn a panicking task, drop the
`JoinHandle`, and the run reports `1 passed`. So panic probes are useless exactly where transport
bugs live.

*Boundary, which all three share (Ren):* **green means "not executed within the window W the test
voluntarily waits", and W is usually undeclared and often zero.** A task that aborts after 50 ms,
not awaited, yields `1 passed, finished in 0.00s` — the runtime dropped, the task was cancelled, the
abort never ran. If reaching the probe takes any time at all, green is guaranteed. W belongs to the
test, not to the probe, and the coupling is inverted: when a test's assertions depend on the path,
W is wide and the probe is reliable, but then you barely needed to ask; when they do not, W is
approximately zero, which is when you most want to ask.

*Consequence (Ren):* when W is approximately zero the answer is not a better probe, it is to widen W
first — join, await, or wait for an effect the task must produce. **There is no honest binary
"unreachable" across threads**, only "not observed within W". Same move as replacing "passed" with
"Serving in 1s of a 20s budget": a binary conclusion that hides an undeclared parameter, replaced by
a quantified one.

## 17. A circular fixture is blind to the drift it exists to catch

*Origin (case Cindy, category named by Ren):* `runtime.rs` classified an invalid join ticket by
matching the error text against a literal, and the test fed that same literal in. It verified that
the matcher works on the string the test supplies — not that the string it supplies is the one
production emits. Reword the producer and classification silently degrades to a generic class while
the unit test stays green.

**A test that constructs the input it should have obtained from production code is blind to that
code's drift.** This is the test-side dual of *assert the symptom, not the cause*: one says do not
hardcode the cause, the other says do not manufacture the system's own output.

*Fix, and the two halves it does not fix (Cindy; second half per Tess):* a shared exported constant
removes drift **between the producer and consumer it binds**. It does not, by itself, establish either
of the following:

    that no other literal exists      three separate things, none of which the constant supplies:
                                      (i) the enumeration's SCOPE — what was searched, and how many
                                          sites that came to; this is the denominator, not a control
                                      (ii) a POSITIVE CONTROL — the search must hit a target known
                                          in advance to be present (the constant's own definition
                                          serves); a search that can never match anything still
                                          reports a scope
                                      (iii) the result — no second literal outside the bound pair
    that production reaches the branch  that needs driving the real failure end to end

Keep all three claims separate in the commit message, or "the constant landed" gets read as "uniqueness
proven" and "the production path is verified" — and the next person will extract a constant and declare
the uniqueness argument done.

*Related, on binding duplicated literals (Ren):* bind only those whose drift degrades **silently**.
Same ruler, three answers — a producer/consumer pair that drifts to a generic class with unit tests
green must bind; an E2E script hardcoding product error text goes red immediately on drift, so
duplication is acceptable; a string with no consumer at all has nothing to degrade.

## 18. An uncharacterised failure cannot be closed by any number of greens

*Origin (Cindy and Ren, on two live cases):* a Chaos run showed three nodes stuck as candidates for
about 20 seconds. The suspected cause — that a new keepalive covers it incidentally — was explicitly
labelled a hypothesis. Later the full matrix passed on the release head.

Not "a later green is weaker evidence than an earlier red"; that invites reading it as a weight
comparison. **The red happened under a condition set we still cannot state; the green happened under
the condition set the matrix defines; the relation between those two sets is unknown.** The green is
therefore not weak evidence about that red — it is not evidence. A shot that missed the bullseye
cannot tell you where the bullseye is.

    red characterised    → you can check whether the matrix covers those conditions → a green can close it
    red uncharacterised  → the relation cannot be decided from the evidence now available
                           → no quantity of unrelated greens closes it

**So "uncharacterised" is a logical state, not a backlog state.** *Run enough greens and treat it as
gone* is not a discouraged third option; it does not exist.

*Boundary (Tess):* uncharacterised does not mean never investigate. It means the investigation owes a
reproduction or a characterisation, and until then the item travels with the release rather than
being closed by it. Record it in the form that helps on recurrence — who owns it, and what the
comparison baseline is — because "fixed" sends the next person hunting through new code that was
never shown to be causally related, exactly when the scene is fresh and most valuable.

## 19. A command can answer a narrower question than the one you asked

*Origin (Cindy, three instances in one day; Ren, two):* a `grep -c` inside a `for` loop whose exit
code swallowed the real result and printed `0` for both heads when one had the thing; a
quote-delimited pattern that missed an embedded occurrence, so "four literals" were five; and
`grep -c 'a|b'`, which on this machine returns a well-formed **0** because `grep` is ugrep and a bare
alternation is literal there.

This is not "the check did not run" — that category is already named, and it at least produces no
output. **Here the check ran, succeeded, and returned a well-formed answer to a narrower question
than the one intended.**

*Criterion, executable at the moment of writing:* state what this command actually answers, then
compare it to what you meant to ask.

*Boundary (Tess and Ren):* the rule is not "use broader patterns". @Ren's first audit for this very
category used `grep -n grep | grep '|'` — wide enough to catch shell `||`, six well-formed hits, none
of them the thing. **Neither width nor narrowness is the test; "which question does this answer" is.**
It was committed under ten minutes after the category was named, by the person auditing for it — the
rule does not defend against ignorance, but against an error that happens while you are thinking
about it.

*Why this is not folded into rule 12 (Ren):* the two are told apart by their remedy. Rule 12 —
the tool is broken — sends you to verify or replace the tool. Rule 19 — the tool is fine and answered
a different question — sends you to restate the question. Merging them makes a reader apply rule 12's
remedy to rule 19's fault: re-checking whether `grep` is installed correctly, when `grep` was never
the problem.

*Corollary (Cindy):* the auditing tool must not share the defect under audit. Scanning the repo for
ugrep-incompatible grep patterns was done with python, not grep. And the audit's own clean result
needed a positive control — 0 findings is indistinguishable from "my regex matched no grep calls at
all", so report how many were examined.

## 20. When you measure an absence, the instrument must not share the property you are measuring

*Origin (Cindy, three instances; both boundaries by Ren):* an instrument that shares the failure
mode under investigation cannot detect it, and it fails in the one way that looks like success:
**it returns "nothing found", which is exactly what a working instrument returns when there is
genuinely nothing there.**

    proving a test suite has no coverage   the oracle must sit outside the test suite — the compiler
                                           reporting `constant is never used`, not another test
    auditing grep's alternation handling   audit with python; grep would walk into the very
                                           alternation handling under audit
    confirming a mechanism was removed     the compiler is an independent witness; `git diff --stat`
                                           only restates my own edit

*Boundary 1 — independence is scoped, not absolute (Ren):* the same instrument can be sound in one
context and unsound in another. The `panic!` probe of rule 15 self-reports on the test thread's stack
and not across threads (rule 16). Changing context without changing instrument is the cheapest way to
break this rule.

*Boundary 2 — the control is itself subject to rule 19, and this is where the regress stops (Ren):*
every "nothing found" needs a positive control, but a control can answer a narrower question than
"did this instrument discriminate here". Two failures show the two directions it must cover:

    my grep     the invocation regex excluded `|` — the very character the check looked for — so the
                alternation check never executed. Constant zero, dressed as a result.
    Ren's grep  the option test was `" -E " in seg`, missing the merged cluster `-Eiq`, so it fired
                on something already safe. A false positive.

**Two directions are necessary but not sufficient.** The control must exercise **every form the corpus
actually contains**; two directions is the special case of a single-form corpus. Ren found this while
testing the rule: his form census reported `--all` and `--check` present, which would have voided his
own clean result — until he read the line and saw they were `cargo` flags on a line that also
contained `grep`. His census answered *which flags appear on lines containing grep*, not *which flags
grep received*.

**The regress terminates, but only against a declared scope (Tess).** "Forms are finite and enumerable"
holds for a corpus you have named, at an exact head — not in general. And "read the matching lines" is
not yet a floor, because the same narrow matcher may be choosing which lines you read:

    1. declare the corpus and the exact head it is taken at
    2. enumerate every actual invocation in that scope by means that do NOT depend on the property
       under measurement
    3. inspect each one directly, deciding which arguments belong to what

The floor is **every concrete instance in the enumerated scope**, not whatever a matcher returned.
**If you cannot show the enumeration is complete, narrow the claim to the forms you actually
observed** — you may not say "every form".

*Not a generalisation of rule 15 (Ren):* rule 15's third premise is about **reachability** — does the
test execute this line — while this rule is about **discrimination**. Merging them collapses two
different properties.
