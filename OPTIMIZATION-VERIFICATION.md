# Allocation and cache optimization — 2026-11-08

Subsequent user-requested lyric zoom replaces this pass's fixed-size text with bounded quarter-pixel font steps. See [the later verification report](SPACING-HOVER-VERIFICATION.md). The measurements below remain historical observations, not benchmarks of that later build.

**RAM is still outside the original targets.** This pass removes avoidable allocation/cache work; it does not establish a large reduction in steady memory.

Implemented:

- Lyric movement keeps one 18px text size and retains continuous scrolling/fading. Femtovg keys rasterized glyphs by font size quantized to 0.1px; the former 13–18px animation could populate dozens of size variants per glyph. Fixed size prevents that churn without changing motion or transparency.
- Media timeline updates reuse the player model when its catalog is unchanged, avoiding repeated row/model allocations and chooser rebuilds.
- Disabling online lyrics releases current/pending documents and asynchronously purges the bounded cache. The sleeping worker retains provider cooldown, preserving Retry-After. Cleanup is idempotent and does not repeatedly wake the worker for disabled-state updates.
- Fixed a shutdown race found during repeated testing: the worker could consume its stop wake while coalescing commands, then block again. It now checks stopping before waiting again. The resource test exercises 100 creation/cleanup cycles.

## Release measurements

Solid material, 100% size, isolated preferences, previews disabled, eight-second warm-up followed by about eight seconds of sampling. Private bytes and working set use System.Diagnostics.Process counters and are reported in MiB (1,048,576 bytes); working set is resident memory, not private allocation. Online lyrics were enabled for the main comparison. The Media sample uses the available real publisher, so these separate process runs are observations, not controlled causal allocation profiles.

| State | Before private MiB | After private MiB | Before working set MiB | After working set MiB |
| --- | ---: | ---: | ---: | ---: |
| Idle | 77.30 | 78.43 | 80.24 | 80.49 |
| Media | 101.15 | 100.27 | 100.08 | 100.23 |

Both runs recorded 0% process CPU at counter resolution. This is a short steady-state observation, not proof of zero CPU/GPU work. The synthetic lyric-animation/capture run peaked at 98.02 MiB private / 86.65 MiB working set before and 98.88 / 88.96 after. No peak-memory improvement was demonstrated. A separate pre-change run with online lyrics disabled sampled 100.08 MiB private in Media; online lookup alone does not explain the roughly 24 MiB increase over idle.

The after Media sample is approximately 105 MB in decimal units, exceeding the 100 MB ceiling, and the preferred 30/45 MB idle/expanded targets are also missed. Most of the baseline already exists before a lyric request. Exact attribution among graphics, font, native service, and allocator resources requires heap/driver profiling; this pass did not perform that profiling or hide the gap by trimming working set. No renderer, transparency, hit testing, or spring quality was sacrificed.

Raw results: `verification/optimization-before.csv`, `optimization-after.csv`, `optimization-animation-before.csv`, `optimization-animation-after.csv`, and `lyrics-no-network-performance.csv`. The final worker shutdown recheck was added after these samples; it changes shutdown synchronization, not the measured rendering path.

## Verification

140 workspace tests (including 100 resource cleanup cycles), warning-denied Clippy, formatting, release compilation, and the native lyric regression were run. The lyric regression passed intermediate movement, interruption, pause/seek, hit routing, and no further renderer frames after settling. The live-provider test remains opt-in. No new dependencies, busy loops, worker threads, or persistent high-frequency timers were introduced.

```powershell
cargo test --workspace --locked --quiet
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --locked
powershell -File scripts/measure-lyrics.ps1
# Diagnostic comparison; does not change normal preferences:
powershell -File scripts/measure-lyrics.ps1 -DisableOnlineLyrics
```
