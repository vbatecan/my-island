# Acrylic clipping and glass responsiveness

## Changes

Acrylic now uses a blurred desktop texture clipped to the same contour as the island. Whole-window DWM material and full-client frame extension are disabled; they were painting the large persistent canvas rather than just the visible shape. Acrylic has no refraction and retains the configured tint. It refreshes on relevant desktop/window events.

Eco/Acrylic GDI capture runs off the UI thread, with at most one capture task plus one coalesced follow-up. Configuration epochs reject stale results. No polling or persistent sampling timer was added.

The CPU compositor bounds backdrop preprocessing to384×240 and optical images to approximately128k pixels independently of DPI. Text and vector geometry retain native resolution. Analytic contour distances match the shared contour, flat-center optics are hoisted, unchanged reduced backdrops skip blur/upload, and frosted/refractive caches remain distinct.

Live capture uses the same bounded optics and retains its30Hz backdrop ceiling independently of the spring timer. The default is the lower-memory copy device, with hardware fallback. A developer --capture-hardware option permits driver comparisons.

## Verification

- Workspace tests:205 passed,2 ignored (optional live lyrics lookup and interactive native DWM test).
- Native DWM/region test explicitly run: passed. Readback confirms no whole-window backdrop, and native polygon/hit regions follow idle, expanded and compact contours at130% size.
- All14 material tests passed, including25,600 analytic-versus-shared-contour points, DPI/canvas mapping, alpha coverage, bounded work, cache separation and unchanged-frame detection.
- Both snapshot-scheduler tests passed:100-event bursts retain one worker/one follow-up; material changes invalidate old frames without spawning another worker.
- Workspace check, formatting, Clippy with warnings denied and release build passed.
- Acrylic release screenshots inspected; snapshot checks validate transparent canvas corners and silhouette bounds.

## CPU compositor benchmark

Optimized same-process helper,560×400 panel and1040×647 backdrop, uncached alternating geometry:

| Scale | Old lens ms | New lens ms | Frosted ms |
| --- | ---: | ---: | ---: |
|1.3|127.803|4.219|3.130|
|2|289.406|4.155|3.109|
|3|1015.133|4.300|3.103|

Backdrop preprocessing was about16–17ms previously and4.3–5.0ms after reduction. Raw output: target/glass-cpu-bench/results.txt. These are CPU compositor timings, not displayed FPS. Background compilation affected baseline timing; no controlled GPU benchmark was performed.

## Whole release application

Fresh isolated profiles, solid test preferences except selected material,130% island size. scripts/measure-material.ps1 drives finite native transitions without global input; raw samples include private bytes, working set, process CPU and callback timing. Desktop background activity was not controlled.

| Material/run | P95 animated render callback interval | Peak sampled private MiB | Peak sampled working set MiB |
| --- | ---: | ---: | ---: |
|Old Eco glass|120.861ms|126.81|113.49|
|Optimized Eco glass|14.652ms|122.90|107.74|
|Optimized Acrylic|13.399ms|120.67|105.70|
|Optimized Live, hardware-preferred|16.217ms|175.03|131.19|
|Final Live, low-memory default|13.000ms|130.80|118.30|

Callback intervals are not presentation FPS. The rapid-interruption phase consumed89.08%,93.80%,60.41% and70.73% of one logical core respectively. Eco's CPU usage did not fall in that stress phase because it rendered many more animation frames; its responsiveness improved. Verification timers and changing backgrounds mean these runs do not establish production idle0%CPU/GPU.

Hardware-preferred Live exceeded the150MB private-byte target despite its lower resident working set. The final low-memory configuration measured130.80MiB private (about137.15MB decimal) and118.30MiB working set (about124.05MB), below150MB in this probe, with a13.000ms callbackP95. This is why the final default retains software copies after the main compositor bottleneck was fixed; hardware remains a fallback/comparison option. Rapid interruptions used93.69% of one core, and the settled sample used12.01% while background capture/verification activity continued. No universal150MB guarantee is claimed.

Evidence directories: target/material-verification/baseline-unloaded-glass-3c5824eca3df4f65bbc24ccbad589d98; optimized-glass-bc0a0df6ca114de5804f88fd4e3564d1; optimized-acrylic-232e872918e64883aea6cbb4079c7d03; optimized-live-glass-d2f6b19c688144dfbcabb059b3ccf80a.

## Limits

Acrylic and both glass modes require capture exclusion to avoid feedback and are omitted from normal screen recordings/screenshots; internal renderer verification can still capture them. Backdrops stay in memory during normal use. Acrylic/Eco snapshots do not continuously follow video. Physical multi-monitor/DPI changes, every graphics driver, and actual displayed FPS remain unverified. Large user dimensions and third-party plugins can change memory use.

Final live evidence: target/material-verification/optimized-low-memory-live-glass-536445006f3f44cfa5f19280c45f5fd0. All four optimized material probes completed with the spring timer stopped.
