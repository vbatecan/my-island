# Notch customization and lyrics — 2026-11-08

Contracted and expanded Notch corners each default to 8 logical pixels and are independently configurable from 0–24 in Settings → Appearance (`notch_contracted_radius`, `notch_corner_radius`). Contracted corners apply to Idle and Compact and are capped at half the current height. Floating Island capsules and their 12px expanded corners remain unchanged. Initial startup geometry uses the resolved preference immediately. Preference and display changes retarget the contour without discarding spring momentum or restarting content choreography.

Media expands to 640×216 logical pixels on a sufficiently wide monitor. Lyrics occupy the left column; artwork, player selection, transport, and seeking occupy the right. Monitor/DPI-aware geometry reserves overshoot room. Narrow layouts keep playback controls readable and omit the lyrics column below a 480px destination width. This breakpoint follows the destination, so overshoot cannot flash a second column.

## Lyrics and resource lifecycle

Windows GSMTC supplies track metadata and timeline information, rather than lyrics. Optional automatic [LRCLIB lookup](https://lrclib.net/docs) sends title, artist, album, and duration when the Media panel is open. The Behavior setting (`online_lyrics`) defaults on for this requested integration. No audio recording, fingerprinting, account, or API key is involved.

The service uses the existing Windows dependency and native WinHTTP: system TLS, a fixed HTTPS host, no redirects, identified User-Agent, sequential requests, provider cooldown, cancellable asynchronous I/O, and a five-second deadline per HTTP request. Exact lookup can be followed by one conservative search. Matching requires normalized title/artist equality and duration within two seconds; album matches are preferred. Missing artist/title/duration waits for publisher metadata instead of guessing.

Timed LRC supports repeated timestamps, Unicode, signed offsets, and backward seeking. Synced lines reuse the existing playback clock; pause stops it. Plain lyrics scroll independently of the volume gesture. Instrumental, unmatched, disabled, loading, and unavailable states are explicit. Retry bypasses cached results while respecting provider cooldown. Generation and track checks reject stale results.

Line progression now uses a 300ms native Slint cursor animation. Seven modulo slots preserve absolute line identities while rows slide upward, fade, and change emphasis; adjacent interruptions continue from the interpolated cursor. A pure Rust cursor model distinguishes continuous playback from initial loads, large/backward seeks, invalid positions, and disabled motion. Rows change only when the active line changes, including blank lines. No extra playback timer or worker was introduced. Existing animations/reduced-motion preferences apply; hiding or disabling motion cancels interpolation. The sync/provider footer has been removed from the panel, with provider disclosure retained in settings.

No lookup starts at construction, while collapsed, or while disabled. Closing/hiding the panel cancels pending requests. The worker blocks on a channel when idle. The memory-only cache is limited to 16 entries and approximately 1 MiB including retained allocations; responses are capped at 256 KiB and parsed text at 128 KiB per document. Metadata reads avoid artwork cloning. Unchanged artwork is deduplicated, absent artwork releases the UI image, and retained artwork is smoothly resized to a maximum 256px edge without another graphics dependency.

## Verification

Windows release build, Rust 1.99.1, Slint 1.18.1, windows 0.62.2:

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --locked
powershell -File scripts/test-media.ps1
.\target\release\still-island.exe --verify-lyrics --appearance=solid
.\target\release\still-island.exe --verify-contracted-corners --appearance=solid
powershell -File scripts/measure-lyrics.ps1
# Explicit network check; excluded from ordinary test runs:
cargo test --locked services::lyrics::tests::live_provider_metadata_smoke -- --exact --ignored --nocapture
```

139 workspace tests passed; the live-provider test is ignored by default and passed separately during provider implementation, resolving 47 synchronized lines without printing copyrighted lyric text. Tests cover spring behavior, settings migration/persistence, independent radius clamps, responsive geometry and DPI math, lyric progression/seek decisions, matching, parsing, caching, memory limits, cancellation, coalescing, retry, and artwork filtering/cache lifecycle. Formatting, type checks, Clippy with warnings denied, and release compilation passed. The native media probe passed play/pause, previous/next, seeking, selected-player switching, Automatic following playback, and removal fallback using its own silent fixture.

`--verify-lyrics` uses original synthetic lyrics and excludes the live media service and network. Release GUI checks passed for initial/current/previous/next lines, playback advancement, pause, backward seek, default 8px and custom 16px corners, plain fallback, stale generation/track rejection, no result, narrow-width overshoot, and collapse. Animation, playback, and presentation timers stop after settling, with no subsequent animation updates during the idle observation.

The motion regression additionally observes an actual intermediate cursor value and renderer snapshot (`verification/lyrics-moving.bmp`), interrupts and retargets it, checks exact settling, instant seek rebasing, and animation disablement. `--verify-contracted-corners` checks startup 8px, live 3px customization, compact 24px, the 16px idle height cap, independently configured expanded corners, stopped scheduling, and unchanged user preferences on disk. Captures include `notch-contracted-3.bmp`, `notch-compact-3.bmp`, `notch-compact-24.bmp`, `notch-contracted-capped.bmp`, and `notch-expanded-independent.bmp`.

Actual Win32 region and `WindowFromPoint` readbacks accepted both widened body edges and routed both transparent sides to the desktop. Style readback confirmed tool window, nonactivation, topmost placement, no app-window style, and no focus theft. Renderer captures verified transparent canvas corners, solid black interior, silhouette bounds, and top anchoring. Captures were visually reviewed: `verification/lyrics-wide.bmp`, `lyrics-custom.bmp`, `lyrics-plain.bmp`, `lyrics-narrow-overshoot.bmp`, and `lyrics-no-result.bmp`.

Physical runtime checks used the primary 1920×1080 display at 96 DPI. Other DPI scales are covered by mathematical tests, not physical display changes. The narrow layout was exercised through a controlled destination/overshoot fixture. Manual exhaustive edge clicking, physical monitor hotplug, GPU activity, and displayed presentation FPS were not measured.

## Performance baseline before the line-motion polish

Memory measurements use `System.Diagnostics.Process.PrivateMemorySize64` (private bytes) and `WorkingSet64` (resident working set), reported in MiB (1,048,576 bytes). These are distinct metrics; neither is a Rust heap-only measurement.

Before line motion and contracted-corner customization, the release synthetic GUI run peaked at 92.49 MiB private / 86.89 MiB working set, including renderer screenshots. Its renderer callback timings are pre-swap observations, not displayed FPS. The steady release measurements below used `scripts/measure-lyrics.ps1`: an isolated local settings directory, solid material, compact now-playing and automatic previews disabled, eight seconds of warm-up, then eight seconds of sampling. The media sample used the real available publisher without issuing transport commands. Lyrics remained enabled; provider coverage is not inferred from a memory sample. These baseline numbers have not been relabeled as benchmarks of the line-motion build.

| Settled state | Peak private MiB | Peak working set MiB | CPU, one-core % | Sample seconds |
| --- | ---: | ---: | ---: | ---: |
| Idle | 78.68 | 80.09 | 0.000 | 8.131 |
| Media | 101.83 | 100.21 | 0.000 | 8.081 |

CPU is the change in process processor time divided by elapsed time, without dividing by logical processor count. Zero means no increment at the counter's resolution during this sample; it is not a guarantee of zero work forever. These are steady-state samples, not animation CPU or GPU benchmarks. Raw final results are in `verification/lyrics-steady-performance.csv`; screenshot-run samples are in `verification/lyrics-release-memory.csv`.

**The original preferred memory targets and the 100 MB maximum are not achieved by this media sample.** Before the artwork optimization, the same setup sampled 78.76 MiB private at idle and 102.95 MiB in Media; individual process runs vary, so the difference is not a controlled attribution benchmark. The fixed baseline already dominates the bounded lyric text/cache. A read-only observer found 300×300 current artwork (about 0.343 MiB nominal RGBA), so its size alone does not explain the approximately 23 MiB idle-to-media increase. Repeated decoding and retained full-resolution artwork have been removed; finding the dominant remaining renderer/font/native-network allocations requires allocation/driver profiling, which was not performed here. Transparency, hit testing, and animation correctness were preserved. Glass materials were not rebenchmarked in this change.

## Remaining limits

Lyrics require a publisher with usable metadata and a matching provider record. Some recordings have no lyrics or only untimed text; provider timestamps and the publisher's timeline determine synchronization quality. There is no microphone recognition, offline lyric-file importer, word-level karaoke, translation, or manual timing adjustment. Linux backends remain deferred. The original preferred memory targets are goals, not demonstrated guarantees for this Slint/Windows graphics backend.
