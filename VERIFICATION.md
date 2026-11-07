> Historical MVP baseline. See [POLISH-VERIFICATION.md](POLISH-VERIFICATION.md) for current real controls, materials, and measurements.

# Verification — 2026-11-07

## Environment and build

Windows 11 Pro build 26300, primary display 1920 x 1080 at 96 DPI (100% scaling). NVIDIA GeForce RTX 2050, driver 32.0.16.2101. Rust/Cargo 1.99.1, Slint 1.18.1 (minimum Rust 1.92), windows crate 0.62.2. Slint versions and minimum compiler compatibility were checked against crates.io package metadata before implementation.

The release executable is approximately 7.81 MiB. It uses Slint's FemtoVG renderer with native OpenGL composition, without a browser runtime. Only raw-window-handle, Slint, and windows are direct runtime dependencies; their transitive GUI, font, and graphics dependencies remain. No application networking, telemetry, service, or application worker thread is implemented. Backend/driver threads are present; one diagnostic run observed 13 total process threads.

## Completed checks

- `cargo fmt --all -- --check`: passed.
- `cargo check --locked --offline`: passed without warnings.
- `cargo test --locked --offline`: all 9 tests passed.
- `cargo build --release --locked --offline`: passed.
- Release `--verify` run: completed and exited normally. Repeated 100 ms retargets preserved spring continuity, returned to Idle, and logged `active=false` after scheduling stopped.
- Actual integrated HWND: `(750,6)-(1170,226)` for the fixed 420 x 220 canvas; 96 DPI; `WS_EX_TOOLWINDOW=true`, `WS_EX_NOACTIVATE=true`, `WS_EX_APPWINDOW=false`, `WS_EX_TOPMOST=true`. Foreground HWND remained another application.
- Native region and `WindowFromPoint`: island center routed to the overlay; empty canvas corner routed to another window. This verifies cross-process empty-region hit routing, not just an `HTTRANSPARENT` response.
- Separate native smoke harness: offset 0/6/12 and topmost-off readback passed. Integrated styles remain enforced when winit changes window styles.
- Actual renderer snapshots: idle, expanded, media, volume, and settled return. Transparent-corner alpha, opaque black interior, dimensions, and top anchoring all passed. Partial-alpha edge pixels were observed (116 idle, 178 compact volume, 216 expanded). Snapshots were visually inspected; media artwork placement was corrected and recaptured.

Visual artifacts are in `verification/`. The gray background in review BMPs is added by the capture helper for contrast; the alpha-preserving `*-alpha.bmp` files retain the transparent canvas. No desktop screenshot containing unrelated applications is included.

## Release measurements

`performance.csv` and `benchmark-results.txt` contain the deterministic harness measurements. Memory is the maximum sampled within each phase, after excluding the first second of steady phases. Samples use `System.Diagnostics.Process.PrivateMemorySize64` (private committed bytes) and `WorkingSet64` (resident working set, including shared pages). Units below are MiB (1,048,576 bytes), not decimal MB. CPU is measured from process CPU-time differences, with one logical core equal to 100%.

| Phase | Private MiB | Working set MiB | CPU, one core |
|---|---:|---:|---:|
| Initial idle | 57.36 | 64.79 | 0.00% |
| Expanded | 67.89 | 74.02 | 0.00% |
| Media | 68.30 | 73.62 | 0.00% |
| Volume | 74.30 | 79.02 | 0.00% |
| Rapid reversals | 74.83 | 79.50 | 52.74% |
| Settled return | 74.83 | 79.51 | 0.00% |

These measurements include the opt-in harness's 10 Hz orchestration timer and state captures. Reversal CPU measures a deliberately demanding stress workload, not a single ordinary morph. No frame presentation rate was measured. The normal application has no orchestration timer, and its animation timer stops when settled. `scripts/measure-steady.ps1` separately measures each state without captures or orchestration, after six seconds of warmup, over a five-second sample window; raw results are in `steady-performance.csv`. Separate-process steady results were: idle 57.35 MiB private / 64.77 MiB working set / 0% CPU; expanded 67.34 / 73.59 / 0.27%; media 67.04 / 73.30 / 0%; volume 63.71 / 71.77 / 0%. This small nonzero expanded sample illustrates why approximately-zero idle CPU is a target rather than an absolute guarantee.

GPU counters (`GPU Engine(*)/Utilization Percentage`) were sampled at approximately one-second intervals. Only valid status-zero counters matching this process ID were considered; the metric is the maximum individual engine utilization for each interval, not the sum across engines. `gpu-performance.csv` contains 16 valid in-process intervals, each with 14 valid engine counters. Maximum observed engine activity was **2.0633%** during the reversal workload. Initial idle and multiple stable samples were **0%**; three consecutive samples after settling were **0%**. Two samples after process exit have no matching counter and are recorded as missing, not zero. The first attempt encountered invalid samples in unrelated counters; the corrected collector filters those explicitly.

The preferred 30/45/60 MB memory goals were missed. Both peak private bytes and peak working set stayed below the 100 MB ceiling in this run. Retained caches kept memory elevated after transitions. The renderer, graphics driver, Slint runtime, and Windows/font infrastructure are known contributors, but no allocation profiler was used to attribute their individual shares. Optimization performed: disabled Slint default features, selected one renderer, reused a single small canvas, used procedural artwork, removed idle scheduling, and avoided additional runtime services. Further memory reduction needs allocation profiling; an unvalidated opaque software fallback was not substituted for correct transparency.

## Limits of the evidence

- Testing covered one real monitor at 100% scaling. Other DPI scales, primary-monitor changes, and live display reconfiguration require device testing; the native event handlers are implemented.
- Hit routing was verified at the center and an empty canvas corner. Exhaustive rounded-boundary clicking and interaction with multiple underlying applications were not performed. The window region deliberately includes a one-physical-pixel antialiasing margin.
- Rendered snapshots establish appearance at settled states, not perceived motion quality or a proven 60/120 Hz presentation rate. Spring mathematics, interruption tests, and repeated runtime retargeting passed.
- Manual hover, click, tray menu, Alt+Tab enumeration, and fullscreen game testing were not controlled end-to-end through desktop automation. Taskbar/Alt+Tab behavior is supported by actual style readback. Live pointer input changed an early measurement; final test modes ignore pointer input for deterministic results. Normal application input remains enabled.
- No fullscreen suppression, real audio control, media-session integration, or startup registration is implemented. Settings and clean tray-exit code are present; the harness directly exercises event-loop exit rather than clicking the tray menu.
- Windows UI automation was attempted but unavailable through the provided surface. No manual UI verification is claimed from that attempt.

Earlier failed startup attempts and pointer-contaminated runs are excluded from the tables. The unavailable-handle startup failure was fixed by awaiting Slint's native winit window before attaching Win32 integration; window creation attributes prevent activation/taskbar appearance during that interval.
