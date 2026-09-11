# Peer idle watchdog: accepted matched readback

Audit session 77755 exited 0. All 24 cohorts passed: 27,821,713 calls; outcomes {'success': 27821713, 'refused': 0, 'unknown_write': 0, 'read_failure': 0, 'client_rejected': 0}; dropped slots 0.

Values are old accepted5ee → candidate f62 within the same concurrency and repetition. p99 values are histogram bucket bounds in microseconds; percentiles are never averaged.

| API | Workers | Repeat | QPS old → candidate | QPS delta | Mean µs old → candidate | p99 µs old → candidate |
|---|---:|---:|---:|---:|---:|---|
| point_get | 1 | 0 | 26257.16 → 26035.87 | -0.843% | 37.962 → 38.276 | [59.392, 59.903] → [57.856, 58.367] |
| batch_get | 1 | 0 | 26050.75 → 25441.33 | -2.339% | 38.254 → 39.173 | [59.392, 59.903] → [58.880, 59.391] |
| point_get | 64 | 0 | 334590.35 → 332523.07 | -0.618% | 191.146 → 192.335 | [368.640, 372.735] → [344.064, 348.159] |
| batch_get | 64 | 0 | 337318.67 → 326561.29 | -3.189% | 189.568 → 195.816 | [364.544, 368.639] → [352.256, 356.351] |
| point_get | 1 | 1 | 26199.28 → 26252.11 | +0.202% | 38.036 → 37.970 | [60.416, 60.927] → [56.832, 57.343] |
| batch_get | 1 | 1 | 26225.66 → 26002.05 | -0.853% | 38.010 → 38.335 | [60.416, 60.927] → [57.344, 57.855] |
| point_get | 64 | 1 | 342121.03 → 332147.23 | -2.915% | 186.937 → 192.555 | [356.352, 360.447] → [348.160, 352.255] |
| batch_get | 64 | 1 | 338242.13 → 326548.37 | -3.457% | 189.049 → 195.824 | [368.640, 372.735] → [352.256, 356.351] |

Bindings passed: 2331 role source-file checks, 2344 resource samples, 48 fresh drain groups, 48 voter/listener bindings, 80 exited owned lifetimes, 528 retained files / 88,755,271 bytes. Original CPU settings restored exactly; no cleanup error.

Separate same-concurrency/repetition pairs against accepted5ee; not a causal comparison against6707. Whole-call mean and p99 histogram bucket bounds, never averaged percentiles. Five-second shared-host tmpfs quorum-WAL vs memoryRedis diagnostic, not sustained capacity/equal durability. Acceptance validates evidence; candidate selection is separate.
