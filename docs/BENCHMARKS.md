# Ayame Benchmarks

*日本語版: [ja/BENCHMARKS.md](ja/BENCHMARKS.md)*

Measurement environment: a Linux VM with 4 vCPUs / 15 GiB RAM; storage is a virtual disk (not NVMe-class).
The numbers come from a modest environment — real NVMe hardware will be faster.

To reproduce:

```sh
cargo build --release
./target/release/ayame gen huge.csv --lines 300000000   # generate a synthetic CSV of about 14 GiB
./target/release/ayame stat huge.csv                    # cold index-build time
./target/release/ayame serve huge.csv --port 8800       # measure the API from another terminal
```

## 300 million lines / 14.16 GiB CSV

| Item | Measured | Notes |
|---|---|---|
| Lines / size | 300,000,000 lines / 14.16 GiB (15,205,557,668 B) | ~47 B per line |
| **Cold open + full index build** | **2,349 ms** | mmap + rayon-parallel newline scan. ≈6.0 GiB/s |
| **Index memory** | **2.00 MiB** (73,252 checkpoints) | stride 4096. **0.014% of the file size** |
| **Random single-line access (warm)** | **0.61 ms average** | 50 random line fetches (checkpoint + forward memchr walk) |
| **Full scan** (no-match literal = worst case) | **2.81 s** → **5.03 GiB/s** | one memmem sweep over 14 GiB |
| First-match search (`error`, from the top) | 0.0007 s | |
| Match count (capped at 2000) | 0.036 s | |
| Process resident memory | 2 MiB index + only what is displayed | the 14 GiB body lives in the **evictable OS page cache** (not our own heap) |

Key point: **opening a 14 GiB file on a 15 GiB RAM machine, Ayame itself uses just a few MiB.** The file body sits in the OS page cache, which the kernel can drop under memory pressure. This is the exact opposite of Zed, which uses "in excess of 64GB for a 10GB file" and rejects anything over 6GB.

## Extrapolation to 10 billion lines (the north star)

The index is "one 16-byte checkpoint per 4096 lines", so it is **linear in line count, not file size**:

| Scale | Index memory (theoretical) | Cold index build (at 6 GiB/s) |
|---|---|---|
| 300M lines / 14 GiB | 2.0 MiB (measured) | 2.3 s (measured) |
| 1B lines / 47 GiB | ~6.5 MiB | ~8 s |
| **10B lines / ~475 GiB** | **~40–70 MiB** | **~80 s** |

- Random line access stays **sub-ms** regardless of scale (always at most a 4096-line memchr walk from the nearest checkpoint).
- Incremental indexing — so browsing can begin from the already-indexed head while the build runs — is on the roadmap (§ DESIGN.md).
- On real NVMe (several GB/s), build time stays I/O-bound around the figures above even without page-cache hits.

> These are **single-process cold** measurements from v0.1. Once the disk cache from `DESIGN.md` (Step 2) landed, second and later opens become "mmap + verify" instead of "build" — near-instant.

## Persistent index cache (Step 2)

| Operation | Time |
|---|---|
| First open (3M lines / 135 MiB, index build) | 24 ms |
| Subsequent opens (mmap + checksum verification) | **0 ms** |

The cache blob holds only the index (11.5 KiB for 3M lines). When the source changes, a size/mtime mismatch auto-invalidates it and the index is rebuilt.

## External merge sort (memory-bounded, disk spill)

`ayame sort` generates runs under an explicit memory budget, spills the excess to disk, and k-way merges. The point is that it completes reliably even with a **budget ≪ data size**.

| Data | Budget | Runs | Spill | Time | Verification |
|---|---|---|---|---|---|
| 5M lines / 244 MiB (column 5, numeric ascending) | **16 MiB** | 15 | 95.4 MiB | **3.25 s** | ordering output |
| 1M lines / 48 MiB (column 5, numeric ascending) | **8 MiB** | 6 | 19.1 MiB | <1 s | passes `sort -c -n` |

- The budget (16 MiB) is **1/15** of the data (244 MiB). All keys cannot fit in RAM, so **spilling necessarily occurs** — and the sort is still correct: true out-of-core.
- Memory residency is roughly "budget + heap proportional to run count", independent of file size.
- The result is a **permutation of line numbers** (a `u64` column). In the future, the editor will walk this permutation through the existing sparse fetch and display **the sorted result without copying it**.
