# Netem drop counter reporting correction

The earlier Chaos reports summed the root qdisc and its netem child's drop
counters. The same discarded packets appeared at both levels, so those sums
overstated the number of netem drops by a factor of two in these records.
The corrected numbers use only the selected `netem 5:` child, parent `1:4`.

| Runtime | Previously reported sum | Netem leaf before | Netem leaf after | Correct delta |
|---|---:|---:|---:|---:|
| `850f0de` | 376 | 2 | 190 | **188** |
| `af4c4e3` | 332 | 2 | 168 | **166** |

These original runs still have positive injected-loss evidence because the
named netem leaf itself advanced. Their TCP retransmission counts, selected
paths, complete-history witnesses, window outcomes, cleanup and performance
results are unchanged. A parent-only increase would not establish netem loss;
the arithmetic correction does not endorse that weaker inference.

Original raw captures, summaries, result documents and gate verdicts remain
unmodified. The current repository prose corrects the historical count; this
is a read-only reinterpretation of retained data, not another fault run.

| Raw capture | SHA-256 |
|---|---|
| `/tmp/kv9-native-link-acceptance-850f0de-attempt1/commands/command-1151.stdout` (before) | `1522c40d7e10b6facd9d1a8020a6dda39e1f44499ae4bbad49492ab42bd673a0` |
| `/tmp/kv9-native-link-acceptance-850f0de-attempt1/commands/command-1335.stdout` (after) | `47e76267db1025d3a8faebb0e30b31c78593d736ab4df6f3540d955abc35c4f2` |
| `/tmp/kv9-native-link-acceptance-af4c4e3-attempt1/commands/command-1152.stdout` (before) | `d19ba6d796369fa1cae9cdb0f2db0205ff8fe05cc048edb94a008dd037e47232` |
| `/tmp/kv9-native-link-acceptance-af4c4e3-attempt1/commands/command-1336.stdout` (after) | `b694610569d81ba883479bcfd99467abf49d115489667c13870ae127ebc5154a` |

The bounded correction record, including exact commands and leaf blocks, is
`/tmp/kv9-native-fault-extension-preparation/drop-counter-correction.json`,
SHA-256 `f5d91e8f217088e95b57fbf4d453a92e32b48e18242c3c2809f69e3f6aab25f6`.
