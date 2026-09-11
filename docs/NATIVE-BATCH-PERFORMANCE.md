# Native batch baseline: throughput and whole-batch latency

The first repeated paired baseline confirms that the batch integration candidate
still trails Redis. At 64 concurrent batches, batch-1 reads deliver
110,052–115,034 successful batches/s with a 1.131–1.262 ms p99 envelope;
Redis MGET delivers 486,386–507,451 batches/s with 0.229–0.234 ms p99.
At batch 256, KV9 reaches 3.486–3.505 million read items/s, but whole-batch
p99 is 9.044–9.699 ms. Batch-256 writes reach 651,578–652,800 items/s
with 33.030–35.127 ms p99. These are not Redis parity or point-API results.

All 144 cohorts contain 28,760,002 successful calls in total, with no refused,
unknown-write, read-failure or client-rejected calls. The original reports,
including every outcome population, remain retained. No cohort hit its cap.
At this source revision the batch read handler still uses the synchronous
established-read path, while point GET already has asynchronous quorum/resident
preparation. A later [matched point/batch-1 comparison](POINT-BATCH1-PERFORMANCE.md)
measures the async batch fix at `af4c4e3`; the larger-batch and write results
below retain this original revision and must not be attributed to that fix.

The independent readback accepted all 144 original inner cohorts. The original outer wrapper exited **1** because Docker did not restore an empty configured cpuset; `restoration_complete=false` remains unchanged. A separate repair restored effective CPUs 0–31 using an explicit `0-31` setting. Original empty-string configuration was **not** restored. Container identities and historical namespace UIDs were preserved.

This is a shared-host diagnostic at clean `3bd1751bddf9a85b07447b64a12adf28ed0df8a9`: three KV9 voters use normal quorum/WAL/sync behavior on volatile tmpfs; Redis uses standalone memory with save/AOF disabled and no replicas. It is not a durability-equivalent or point-control comparison. Three project-owned containers were confined to background CPUs; unrelated BuildKit, host daemons and interrupts were not isolated. Measured clients used CPUs 0–1 and servers 2–5.

Every cell uses two order-balanced 1,500-ms repetitions, 128 warmup calls, 4,096 keys, 128-byte values, a six-character run ID (23-byte keys), and a 1,500-ms call deadline. No cap was hit. All unknown-write, refusal, read-failure and client-rejection populations were zero in all 144 cohorts; every original population and reason count remains in `statistics.json`. Fixed offered-load curves have not been run.

Audit evidence: 575 source hashes; 72 paired configurations; 432 unique owned lifetimes exited; 4,173 in-cohort resource samples; 216 qualifying two-export drains; 216 voter/listener/mount writer bindings; and all 3,108 retained files / 27,014,529,404 bytes matched their original tmpfs-retention hashes.

Ranges below are the minimum–maximum of the two repetitions. Latencies are for **one whole batch**, in microseconds; they are never divided by batch size. Mixed read/write latency merges actual histogram sample counts/sums and recomputes quantiles. The p50/p95/p99 columns are envelopes of the two repetition bin intervals, so they include both histogram precision and repetition spread. All-terminal and success-only latency coincide because failures were zero.

| Batch | Mix | Workers | System | Batches/s | Items/s | Mean μs | p50 μs envelope | p95 μs envelope | p99 μs envelope |
| ---: | --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | read | 16 | KV9 | 86,144.8–86,326.9 | 86,144.8–86,326.9 | 185.2–185.6 | 178.2–180.2 | 270.3–274.4 | 327.7–331.8 |
| 1 | read | 16 | Redis | 486,995.7–499,283.6 | 486,995.7–499,283.6 | 31.9–32.7 | 30.7–31.2 | 43.0–44.0 | 50.2–53.8 |
| 1 | write | 16 | KV9 | 63,844.5–63,957.2 | 63,844.5–63,957.2 | 250.0–250.5 | 243.7–245.8 | 344.1–348.2 | 401.4–409.6 |
| 1 | write | 16 | Redis | 490,827.6–493,102.1 | 490,827.6–493,102.1 | 32.3–32.5 | 31.2–31.7 | 39.9–42.0 | 48.1–50.7 |
| 1 | mixed | 16 | KV9 | 63,891.8–64,069.0 | 63,891.8–64,069.0 | 249.6–250.3 | 241.7–245.8 | 352.3–356.4 | 413.7–417.8 |
| 1 | mixed | 16 | Redis | 489,865.7–492,546.1 | 489,865.7–492,546.1 | 32.3–32.5 | 31.2–31.7 | 42.0–44.0 | 50.2–50.7 |
| 1 | read | 64 | KV9 | 110,051.7–115,034.1 | 110,051.7–115,034.1 | 556.1–581.3 | 524.3–548.9 | 917.5–983.0 | 1,130.5–1,261.6 |
| 1 | read | 64 | Redis | 486,386.2–507,451.4 | 486,386.2–507,451.4 | 126.0–131.4 | 118.8–121.9 | 170.0–176.1 | 229.4–233.5 |
| 1 | write | 64 | KV9 | 92,704.1–93,630.8 | 92,704.1–93,630.8 | 683.3–690.1 | 663.6–671.7 | 950.3–983.0 | 1,114.1–1,179.6 |
| 1 | write | 64 | Redis | 492,957.7–499,005.0 | 492,957.7–499,005.0 | 128.1–129.6 | 120.8–122.9 | 159.7–165.9 | 235.5–237.6 |
| 1 | mixed | 64 | KV9 | 91,310.5–91,320.9 | 91,310.5–91,320.9 | 700.5–700.6 | 671.7–679.9 | 1,024.0–1,040.4 | 1,212.4–1,245.2 |
| 1 | mixed | 64 | Redis | 496,622.4–502,883.3 | 496,622.4–502,883.3 | 127.1–128.7 | 119.8–121.9 | 165.9–172.0 | 229.4–235.5 |
| 4 | read | 16 | KV9 | 78,029.5–78,766.9 | 312,117.9–315,067.8 | 202.9–204.8 | 194.6–198.7 | 303.1–311.3 | 364.5–380.9 |
| 4 | read | 16 | Redis | 449,132.5–455,161.7 | 1,796,530.1–1,820,646.9 | 34.9–35.4 | 33.8–34.3 | 47.6–50.7 | 57.3–60.4 |
| 4 | write | 16 | KV9 | 46,119.1–46,403.0 | 184,476.5–185,612.2 | 344.7–346.8 | 331.8–335.9 | 483.3–487.4 | 573.4–581.6 |
| 4 | write | 16 | Redis | 418,892.1–420,023.6 | 1,675,568.3–1,680,094.5 | 37.8–37.9 | 35.8–36.9 | 51.2–53.2 | 55.3–56.3 |
| 4 | mixed | 16 | KV9 | 50,470.7–50,569.0 | 201,882.7–202,276.0 | 316.2–316.8 | 303.1–307.2 | 454.7–458.8 | 540.7–548.9 |
| 4 | mixed | 16 | Redis | 424,753.7–433,794.2 | 1,699,015.0–1,735,176.8 | 36.6–37.4 | 35.3–36.4 | 50.7–54.3 | 57.9–59.4 |
| 4 | read | 64 | KV9 | 98,790.0–101,404.9 | 395,159.9–405,619.6 | 630.7–647.4 | 589.8–614.4 | 1,065.0–1,114.1 | 1,343.5–1,392.6 |
| 4 | read | 64 | Redis | 457,690.3–465,407.3 | 1,830,761.0–1,861,629.2 | 137.3–139.6 | 131.1–133.1 | 170.0–204.8 | 229.4–282.6 |
| 4 | write | 64 | KV9 | 61,271.2–61,731.7 | 245,084.6–246,927.0 | 1,036.4–1,044.1 | 999.4–1,007.6 | 1,474.6–1,490.9 | 1,785.9–1,835.0 |
| 4 | write | 64 | Redis | 416,604.0–423,410.9 | 1,666,416.0–1,693,643.7 | 150.8–153.3 | 143.4–145.4 | 192.5–206.8 | 247.8–251.9 |
| 4 | mixed | 64 | KV9 | 68,062.4–68,259.5 | 272,249.8–273,038.0 | 937.0–939.7 | 892.9–909.3 | 1,376.3–1,392.6 | 1,671.2–1,703.9 |
| 4 | mixed | 64 | Redis | 433,962.5–439,473.4 | 1,735,849.8–1,757,893.7 | 145.3–147.2 | 139.3–141.3 | 186.4–217.1 | 237.6–241.7 |
| 16 | read | 16 | KV9 | 60,842.8–61,064.5 | 973,484.5–977,032.5 | 260.8–261.8 | 247.8–251.9 | 393.2–397.3 | 479.2–483.3 |
| 16 | read | 16 | Redis | 255,228.1–258,818.4 | 4,083,649.4–4,141,093.9 | 61.1–62.0 | 60.4–60.9 | 89.1–91.1 | 98.3–107.5 |
| 16 | write | 16 | KV9 | 23,888.4–23,925.8 | 382,214.5–382,812.2 | 668.5–669.6 | 630.8–647.2 | 958.5–966.7 | 1,146.9–1,179.6 |
| 16 | write | 16 | Redis | 256,561.7–262,287.1 | 4,104,987.3–4,196,594.1 | 59.7–61.0 | 58.4–60.4 | 86.0–90.1 | 96.3–99.3 |
| 16 | mixed | 16 | KV9 | 30,648.0–30,872.8 | 490,367.3–493,965.3 | 517.5–521.3 | 495.6–499.7 | 761.9–770.0 | 925.7–942.1 |
| 16 | mixed | 16 | Redis | 256,777.7–257,817.8 | 4,108,443.8–4,125,085.4 | 61.0–61.3 | 60.9–61.4 | 90.1–91.1 | 99.3–102.4 |
| 16 | read | 64 | KV9 | 76,943.8–77,948.3 | 1,231,100.8–1,247,173.3 | 819.5–830.2 | 770.0–794.6 | 1,376.3–1,409.0 | 1,736.7–1,785.9 |
| 16 | read | 64 | Redis | 269,951.7–272,251.2 | 4,319,228.0–4,356,019.3 | 234.4–236.3 | 235.5–243.7 | 331.8–340.0 | 352.3–356.4 |
| 16 | write | 64 | KV9 | 27,525.2–27,719.0 | 440,402.9–443,504.4 | 2,308.2–2,323.8 | 2,162.7–2,228.2 | 3,342.3–3,407.9 | 4,259.8–4,456.4 |
| 16 | write | 64 | Redis | 259,915.7–260,747.8 | 4,158,651.4–4,171,964.7 | 244.0–244.8 | 243.7–245.8 | 340.0–348.2 | 368.6–376.8 |
| 16 | mixed | 64 | KV9 | 37,492.4–37,510.3 | 599,878.9–600,164.7 | 1,704.5–1,705.1 | 1,605.6–1,622.0 | 2,555.9–2,588.7 | 3,276.8–3,342.3 |
| 16 | mixed | 64 | Redis | 259,839.2–261,140.1 | 4,157,426.8–4,178,241.2 | 243.9–245.2 | 247.8–249.9 | 348.2–352.3 | 372.7–385.0 |
| 64 | read | 16 | KV9 | 33,251.8–33,429.2 | 2,128,112.5–2,139,465.6 | 473.6–476.2 | 454.7–458.8 | 720.9–737.3 | 868.4–901.1 |
| 64 | read | 16 | Redis | 92,711.6–94,789.5 | 5,933,541.6–6,066,528.1 | 166.9–170.5 | 163.8–170.0 | 247.8–254.0 | 270.3–294.9 |
| 64 | write | 16 | KV9 | 8,537.6–8,593.9 | 546,405.1–550,012.4 | 1,860.7–1,873.0 | 1,736.7–1,753.1 | 2,719.7–2,850.8 | 3,440.6–3,964.9 |
| 64 | write | 16 | Redis | 94,465.5–95,248.5 | 6,045,791.0–6,095,906.6 | 163.5–164.7 | 161.8–163.8 | 243.7–249.9 | 270.3–278.5 |
| 64 | mixed | 16 | KV9 | 12,684.6–12,863.3 | 811,812.3–823,249.1 | 1,240.7–1,258.1 | 1,179.6–1,196.0 | 1,900.5–1,966.1 | 2,359.3–2,621.4 |
| 64 | mixed | 16 | Redis | 91,326.2–91,341.1 | 5,844,876.4–5,845,833.2 | 171.4–171.4 | 170.0–174.1 | 258.0–260.1 | 282.6–286.7 |
| 64 | read | 64 | KV9 | 40,546.6–40,829.4 | 2,594,984.0–2,613,082.7 | 1,561.6–1,572.4 | 1,490.9–1,523.7 | 2,555.9–2,621.4 | 3,145.7–3,244.0 |
| 64 | read | 64 | Redis | 93,268.9–93,986.2 | 5,969,212.7–6,015,119.1 | 678.6–683.8 | 696.3–712.7 | 966.7–991.2 | 1,024.0–1,179.6 |
| 64 | write | 64 | KV9 | 8,726.3–8,785.9 | 558,484.9–562,296.5 | 7,271.3–7,320.4 | 6,946.8–7,012.4 | 10,354.7–11,010.0 | 14,024.7–14,417.9 |
| 64 | write | 64 | Redis | 95,956.1–95,971.9 | 6,141,187.4–6,142,199.0 | 661.7–662.0 | 671.7–679.9 | 942.1–950.3 | 999.4–1,007.6 |
| 64 | mixed | 64 | KV9 | 13,673.4–13,812.4 | 875,099.0–883,992.4 | 4,625.4–4,670.3 | 4,325.4–4,456.4 | 7,340.0–7,602.2 | 10,223.6–10,616.8 |
| 64 | mixed | 64 | Redis | 91,108.4–91,514.8 | 5,830,938.0–5,856,947.6 | 695.1–698.1 | 712.7–720.9 | 999.4–1,015.8 | 1,065.0–1,114.1 |
| 128 | read | 16 | KV9 | 20,722.1–20,909.5 | 2,652,429.3–2,676,410.1 | 754.3–761.2 | 720.9–737.3 | 1,179.6–1,196.0 | 1,409.0–1,441.8 |
| 128 | read | 16 | Redis | 52,527.1–52,724.3 | 6,723,471.9–6,748,705.5 | 300.0–301.0 | 294.9–299.0 | 446.5–450.6 | 479.2–503.8 |
| 128 | write | 16 | KV9 | 4,715.9–4,816.2 | 603,637.0–616,470.8 | 3,319.3–3,389.9 | 3,178.5–3,211.3 | 4,653.1–5,177.3 | 5,963.8–7,209.0 |
| 128 | write | 16 | Redis | 49,352.8–50,023.4 | 6,317,154.7–6,402,998.9 | 310.2–314.4 | 307.2–315.4 | 466.9–475.1 | 516.1–524.3 |
| 128 | mixed | 16 | KV9 | 7,259.8–7,331.4 | 929,255.7–938,417.1 | 2,176.2–2,197.7 | 2,048.0–2,080.8 | 3,375.1–3,506.2 | 4,390.9–5,308.4 |
| 128 | mixed | 16 | Redis | 49,569.9–49,919.6 | 6,344,944.9–6,389,712.1 | 313.3–315.3 | 311.3–319.5 | 471.0–479.2 | 516.1–532.5 |
| 128 | read | 64 | KV9 | 24,788.4–24,830.0 | 3,172,909.3–3,178,234.7 | 2,564.8–2,568.7 | 2,457.6–2,523.1 | 4,194.3–4,259.8 | 5,046.3–5,242.9 |
| 128 | read | 64 | Redis | 52,286.3–52,540.6 | 6,692,642.3–6,725,191.5 | 1,213.5–1,219.4 | 1,261.6–1,294.3 | 1,753.1–1,769.5 | 1,835.0–1,867.8 |
| 128 | write | 64 | KV9 | 4,661.0–4,812.2 | 596,609.6–615,960.9 | 13,249.4–13,692.5 | 12,845.1–13,369.3 | 17,563.6–18,612.2 | 20,447.2–22,020.1 |
| 128 | write | 64 | Redis | 52,332.5–52,763.1 | 6,698,560.6–6,753,672.6 | 1,203.4–1,213.3 | 1,196.0–1,245.2 | 1,720.3–1,753.1 | 1,818.6–1,851.4 |
| 128 | mixed | 64 | KV9 | 7,771.8–7,831.9 | 994,787.9–1,002,488.5 | 8,152.5–8,213.9 | 7,798.8–7,864.3 | 12,714.0–12,845.1 | 14,680.1–15,859.7 |
| 128 | mixed | 64 | Redis | 50,204.9–50,450.8 | 6,426,233.6–6,457,698.1 | 1,260.0–1,266.7 | 1,278.0–1,310.7 | 1,818.6–1,835.0 | 1,933.3–1,966.1 |
| 256 | read | 16 | KV9 | 11,665.8–11,849.5 | 2,986,445.9–3,033,476.6 | 1,331.1–1,352.0 | 1,278.0–1,310.7 | 2,097.2–2,195.5 | 2,555.9–2,621.4 |
| 256 | read | 16 | Redis | 27,621.7–27,973.0 | 7,071,162.1–7,161,091.5 | 564.8–572.1 | 548.9–557.1 | 843.8–860.2 | 892.9–1,024.0 |
| 256 | write | 16 | KV9 | 2,492.6–2,533.3 | 638,097.1–648,533.4 | 6,305.8–6,409.0 | 6,029.3–6,160.4 | 8,781.8–9,306.1 | 11,665.4–12,582.9 |
| 256 | write | 16 | Redis | 26,079.6–26,600.3 | 6,676,387.3–6,809,665.0 | 582.9–594.3 | 573.4–598.0 | 876.5–909.3 | 966.7–991.2 |
| 256 | mixed | 16 | KV9 | 3,986.2–4,045.1 | 1,020,467.5–1,035,547.9 | 3,941.1–3,998.3 | 3,768.3–3,866.6 | 6,029.3–6,488.1 | 8,257.5–9,306.1 |
| 256 | mixed | 16 | Redis | 26,364.1–26,476.0 | 6,749,216.9–6,777,857.8 | 590.2–592.7 | 589.8–598.0 | 892.9–901.1 | 966.7–983.0 |
| 256 | read | 64 | KV9 | 13,618.0–13,690.7 | 3,486,217.3–3,504,816.7 | 4,647.1–4,670.3 | 4,456.4–4,522.0 | 7,471.1–7,733.2 | 9,044.0–9,699.3 |
| 256 | read | 64 | Redis | 27,344.3–27,812.6 | 7,000,149.9–7,120,025.4 | 2,291.6–2,330.4 | 2,359.3–2,490.4 | 3,309.6–3,407.9 | 3,473.4–3,768.3 |
| 256 | write | 64 | KV9 | 2,545.2–2,550.0 | 651,578.0–652,800.0 | 24,947.5–24,991.4 | 24,379.4–24,903.7 | 30,146.6–30,408.7 | 33,030.1–35,127.3 |
| 256 | write | 64 | Redis | 27,803.8–27,848.8 | 7,117,770.9–7,129,287.7 | 2,276.4–2,279.4 | 2,261.0–2,293.8 | 3,276.8–3,309.6 | 3,440.6–3,506.2 |
| 256 | mixed | 64 | KV9 | 4,098.9–4,115.6 | 1,049,306.1–1,053,592.7 | 15,487.1–15,544.4 | 14,942.2–15,073.3 | 23,068.7–24,117.2 | 28,049.4–28,573.7 |
| 256 | mixed | 64 | Redis | 26,069.0–26,172.5 | 6,673,660.5–6,700,155.0 | 2,427.2–2,436.8 | 2,490.4–2,523.1 | 3,506.2–3,571.7 | 3,768.3–3,899.4 |

Inputs: `/tmp/kv9-native-batch-paired-diagnostic-attempt1/cohorts`; original wrapper summary at its parent; repair `/tmp/kv9-native-batch-paired-restoration-repair-first/summary.json`. Audit and original-report hashes are retained in `audit.json` and `input-inventory.json`. No benchmark, workload, build, or control was rerun during this review.

## Exact checkpoint and audit identities

- Runtime/client source: `3bd1751bddf9a85b07447b64a12adf28ed0df8a9`.
- Server SHA-256: `82cf715e6d8d1ea8898db1ad3624431958a21213c50ebc00ae5320943cc03991`.
- Native client SHA-256: `0fcf10b9d644ce1b8323b5736510b0826f5bb89aae3d2c72f2071c5e0b3f5ad2`.
- Redis client SHA-256: `d5c2069f00787a37a14b1cb25848aad05c3d2c684d9609b914becbafc0ec9e2b`.
- Executed paired driver SHA-256: `985370ed3f295462240dbe19ef959739bfc0124da25398dfc46553dff7b32936`.
- Executed isolation wrapper SHA-256: `4062ad936246cc7b249d6a169813a1c3df65f1f3ced6f63812ab9765fc989524`.
- Independent audit SHA-256: `87339fe6d0217e10edb680faa141b2ed12a003e58d6abf236d8cd77f23eb00fb`.
- Per-repetition statistics SHA-256: `f1051c27f612683808ff8d33b3f7c518867701808bd1f99963afcfdfae56b8cb`.

The executable fixtures and clean release builds are retained under
`/tmp/kv9-native-batch-paired-runner-preparation`,
`/tmp/kv9-native-batch-paired-isolation` and
`/tmp/kv9-native-batch-paired-release-build-first`. The exact requested
configuration, source/build manifests, original client report, process/resource
samples, fresh drain/readback and cleanup evidence are retained for every cohort.
The two-target smoke at `/tmp/kv9-native-batch-paired-smoke-first` passed before
the timed run and is not included in these performance statistics.

No hosted CI was dispatched. Fixed offered-load latency curves, directly matched
point controls, remaining proof/Chaos obligations and main promotion remain open
in issue #50. This short same-host loopback matrix does not establish sustained,
cross-host, disk-durable or power-loss performance.
