# Polish verification — 2026-11-07

This report supersedes the demo-only MVP behavior described in `VERIFICATION.md`.

## Implemented

- Real default-output Core Audio volume and mute, external volume notifications, and default-device rebinding.
- Real GSMTC media metadata/artwork, capability-aware play/pause, previous/next, and seeking. No-session and unsupported controls are explicit; the UI does not invent successful transport actions.
- Expanded usable volume panel and compact slider/mute; long-title-safe media layout, cover art, progress, and errors.
- Persisted Solid, Transparent, Acrylic, Eco glass, and Live glass materials; tint presets; Soft/Balanced/Lively spring responses.
- More visible expansion bounce, with stronger collapse damping and preserved interruption momentum. Tests bound the lively response within the canvas.
- Eco glass uses excluded-window GDI desktop sampling on interaction/window events, with no capture worker/timer. Live glass uses optional WGC. Both use rounded SDF normals, vector Snell refraction, a soft backdrop prefilter, bilinear sampling, and Fresnel rim reflection.
- Avoided native region changes when geometry is unchanged, coalesced capture frames, suppressed insignificant changes, and retained event-driven service workers.

## Build and actual integration tests

Windows11 Pro build26300,1920x1080 primary monitor at96DPI, NVIDIA GeForce RTX2050 driver32.0.16.2101. Rust1.99.1, Slint1.18.1, windows0.62.2.

Formatting/check/release build passed; **17 unit tests passed**. Added tests cover preset overshoot ordering/bounds, appearance persistence values, SDF normals, finite normalized refracted rays, normal incidence, bilinear interpolation, filtering invariants, and lens alpha/content. Existing convergence/retarget/timestep/settling/choreography tests remain passing. Release executable is about9.3MiB.

Actual Windows service probes:

- Core Audio read60%, wrote the same60%, readback60%, available=true, muted=false; callbacks and clean shutdown passed. No sound was produced and no intentional volume/mute change was made.
- `scripts/test-media.ps1` passed play, pause, next+metadata, previous+metadata, and seek+timeline through **real Windows GSMTC**. Its hidden native SMTC fixture received five transport callbacks. The session disappeared after fixture closure. The probe refuses commands unless its own fixture is current; no user's media player was controlled and no audio engine was used. Raw results: `media-probe-results.txt`.
- Borderless WGC captured real420x220 RGBA crops without permission prompts; shutdown passed.
- An owned-window GDI fixture showed a blue backdrop and red overlay. Unexcluded capture saw red; after `WDA_EXCLUDEFROMCAPTURE`, capture saw the blue backdrop while the red overlay stayed displayed. This established the Eco path's underlying-desktop sampling without hiding/flickering the overlay.

All five material modes completed release `--verify` sequences, including rapid reversals and return to Idle with animation scheduling inactive. Captures are grouped under `verification/solid`, `transparent`, `acrylic`, `glass`, and `live-glass`. Rendered corners are transparent, dimensions remain anchored, and volume shows the real output value. Live/eco glass captures were visually inspected; backdrop text was softened by the final prefilter.

Native readbacks retained toolwindow/noactivate/topmost styles, absent appwindow style, unchanged foreground focus, and excluded empty canvas corners. Acrylic's DWM backdrop readback was3; other materials used1. Both glass modes used capture affinity0x11. Initial island-center hit routing passed in each mode. One Eco settled sample returned another window at the center despite a valid region/topmost style; a complete follow-up Eco run passed all center/corner samples. The cause of that transient was not established; other topmost windows and controlled manual boundary testing remain relevant. Logs preserve the evidence rather than treating the transient as a passing sample.

## Final release measurements

Raw data: `polish-performance-{material}.csv`; phase summaries: `polish-{material}-benchmark.txt`. Maxima below include **all**100ms samples, including transitions. Private bytes are private committed memory (`PrivateMemorySize64`); working set includes shared resident pages (`WorkingSet64`). MiB means1,048,576bytes. CPU uses one logical core as100%; these short runs include the explicit10Hz verification orchestrator and local snapshot work.

| Material | Maximum sampled private MiB | Maximum sampled working set MiB | Settled-return CPU |
|---|---:|---:|---:|
| Solid |76.18|86.75|1.54%|
| Transparent |76.14|86.69|0.00%|
| Acrylic |77.20|86.60|0.00%|
| Eco glass |78.88|89.48|1.55%|
| Live glass, optimized |79.94|95.06|0.00%|

A follow-up Eco run measured78.71MiB private/89.23MiB working-set maximum and0% CPU during expanded/media/volume/settled phases. Source diagnostics confirmed `active=false` after each settling sequence. A short CPU sample can include service/Windows events; approximately-zero is not a guarantee that every interval is exactly zero.

Rapid100ms reversals consumed about44% one-core CPU in Solid,63% in Eco, and65% in Live. These are stress workloads; actual presentation FPS was not measured. The preferred30/45/60MB memory goals remain missed. All final sampled maxima stayed below100decimalMB on this machine; Live's95.06MiB working set is about99.68decimalMB, leaving very little margin. Actual players/artwork, desktop activity, system policies, drivers, and scales can change this. No general100MB guarantee is made.

The initial hardware-D3D live-glass implementation peaked around126.64MiB private/111.89MiB working set and was too costly. Optimization tested hardware/WARP, pool sizes, cropped backing extents, and dirty reporting. The final path uses **WARP + one buffer + a cropped backing extent + dirty-region checks + a30FPS backdrop ceiling**, independently of morph scheduling. It only publishes when at least32 pixels differ by more than3 RGB levels against the last accepted frame. The isolated probe dropped from61 to17 published frames over8s. Isolated capture-process absolute memory stayed near53MiB, so that number must not be added to the application's baseline as incremental cost; the integrated WARP run demonstrated the actual reduction. No allocation profiler was used to attribute individual heaps. Eco mode avoids that live capture stack entirely.

GPU counters were not rerun for this revision. The original MVP GPU numbers are historical and are not evidence for the new materials. Desktop capture is event-driven, but Live mode must spend CPU/GPU resources when the backdrop changes. Eco deliberately freezes the backdrop between interaction/window events, including when a video underneath continues playing.

## Remaining limits

- Desktop Computer Use discovery was attempted; the native pipe and browser host were unavailable. Controlled manual pointer/tray/Alt+Tab tests could not be performed. Native hit/style readbacks and renderer captures are separate evidence.
- Additional physical DPI scales, display topology changes, device hotplug, other media providers, and fullscreen applications need broader device testing. No fullscreen auto-hide or startup registration.
- DWM can choose an accessibility/power/inactive-policy fallback for native acrylic; accepted backdrop type alone does not prove identical visual blur under every policy.
- Glass is excluded from ordinary screenshots/recordings to prevent self-capture feedback. Internal verifier snapshots still render it. Normal operation retains backdrop pixels only in RAM; explicit verification writes local images.
- Borderless Live capture can be unavailable on other Windows builds/policies; it falls back to Acrylic and reports an error without requesting permission or changing system security settings.
- The application does not implement playback; it controls a real publisher only when that publisher exposes a Windows media session and the requested capability.
