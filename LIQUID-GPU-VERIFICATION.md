# Liquid Glass GPU implementation status

## Appearance and expansion correction

The follow-up removes white tint and white content support from the shader. The clear centre now retains backdrop hue with a small neutral dark tint; directional reflection, an inner shadow and refraction are concentrated on the bezel. Clear and blurred samples are combined to retain backdrop structure.

Expansion uses a stable borrowed output image. The GPU updates its storage in place on the same GL context, so spring ticks do not replace the Slint Image and request an extra frame. An analytic notch/island distance calculation replaces the eight-primitive per-pixel traversal; transparent canvas pixels exit before optical calculations. Texture unit 1 and its sampler are saved and restored alongside unit 0.

The actual renderer fixture now checks dark and saturated pixel values. In both debug and release runtimes, `(20,25,35)` rendered as `(19,24,34)` and `(180,25,95)` as `(175,24,92)`. Both checks passed. Dark, saturated and patterned screenshots were visually reviewed in `target/liquid-gpu-verification/579fa3ad9e7241759eaeef0da37a156c`; final release fixture evidence is in `target/liquid-gpu-verification/892865c363bf4e5eba8955e6c6783cd9`. Workspace tests (200 app tests passed, 2 ignored, workspace plugins passed), strict Clippy, formatting and a serial release build passed for the correction.

Corrected release probes, without compiler contention:

| Material | Mean CPU renderer callback | p95 callback interval |
| --- | ---: | ---: |
| Solid | 2.664ms | 15.695ms |
| Glass | 4.441ms | 36.533ms |
| Liquid Glass (clean repeat) | 2.567ms | 15.509ms |

Evidence: `target/material-verification/liquid-corrected-solid-6688dab74eea4132a4ed3f8bc2644100`, `liquid-corrected-glass-797d063f016a43e79bd02f1966404358`, and `liquid-corrected-repeat-liquid-glass-33cf29176e7144e685053ab2562c767b`. The first Liquid Glass run failed a media-state expectation after an intervening transition to Idle (`liquid-corrected-liquid-glass-0ed4a06672b34bc8a4467e48e81dc197`); it is not counted as a passing probe. The repeat passed all probe checks. These sequential desktop probes are not a controlled GPU/presentation benchmark; the callback intervals do not measure displayed FPS or input-to-presentation latency. They support similar rendering cadence to Solid in this run, not a universal claim that expansion lag is eliminated.

The currently running user's debug executable predates the GPU work (build timestamp 14:06 on 2026-11-10). It has not been stopped or replaced. The corrected release executable must be launched after exiting that instance.

The OpenGL compatibility path is implemented. This is **not completion of the full Liquid Glass plan**, and the latency targets have not been established.

## Implemented

- Slint 1.18.1 / femtovg retained; one main-window rendering notifier dispatches material setup, rendering, metrics and teardown.
- Liquid Glass no longer uses the CPU lens rasterizer or baked white pixel lifting. Other CPU materials retain their compositor.
- Context-owned shader, framebuffer, VAO, source/blur textures and one stable borrowed output texture; storage changes only with dimensions. Output is capped at 1,048,576 pixels.
- Analytic shared notch/island contour equations include concave shoulders. Separable linear-light blur, squircle bezel, IOR 1.5 refraction, 8px bezel, 6px virtual thickness/displacement limit, Schlick reflection, and a small neutral dark tint.
- Slint's actual 1.18.1 borrowed-texture importer expects straight RGBA; the shader leaves coverage multiplication to Slint, producing premultiplied output without double coverage multiplication.
- Content-region linear luminance sampling with 150ms filtering and 0.35/0.55 hysteresis. Mixed light foreground content gets feathered dark support limited to 15%; white support has been removed.
- Main window and settings preview use the same shader with separate context-owned resources.
- Hardware-first WGC, dirty-region rejection, three reusable nonblocking staging slots, GPU mip downsampling to at most 262,144 uploaded pixels, worker-side change digest, latest-frame UI mailbox and generation rejection.
- Capture startup and teardown no longer wait on the UI thread. Finite worker wakeups drain pending slots; capture device/readback failure routes to existing snapshot recovery without changing the selected Liquid Glass setting.
- Explicit isolated verification instance support; no global keyboard or mouse injection and no termination of the user's running instance.

## Evidence so far

- `cargo check --workspace` passed.
- `cargo test --workspace -j 1`: 200 app tests passed, 2 ignored; all workspace plugin tests passed.
- `cargo clippy --workspace --all-targets -j 1 -- -D warnings` passed.
- `cargo fmt --all -- --check` passed.
- A normal release build completed. An earlier concurrent build ran out of memory; serial compilation succeeded.
- Isolated debug runtime: `scripts/test-liquid-gpu.ps1 -Executable target/gpu-optics-fixture.exe` passed. Evidence in `target/liquid-gpu-verification/0cb40c4b4b5f4cdbb26ee29adc1bfbd8` includes seven real renderer snapshots and native region/hit-test diagnostics. Bright, dark and patterned images were visually inspected; canvas corners remained transparent. Both forms, enlarged media text, compact, battery, timer and expanded states were exercised.
- The fixture's initial capture log only indicates that a worker exists, not that hardware capture completed. The fixture then supplies owned sample backdrops, so it does not establish live capture latency or device recovery.
- Final serial release build passed on the final source. The final release fixture passed with evidence in `target/liquid-gpu-verification/e0440ba0977644529acc9d9223427ff9`; the patterned enlarged media view was also inspected.

The final release probes ran sequentially with no compiler running. The existing user instance remained running and the desktop was not a controlled moving-backdrop fixture.

| Material | Mean CPU renderer callback | p95 callback interval | Settled one-core CPU sample | Settled private memory |
| --- | ---: | ---: | ---: | ---: |
| Solid | 1.608ms | 11.919ms | 0.00% | 119.21MiB |
| Live Glass | 2.074ms | 20.320ms | 10.10% | 176.75MiB |
| Liquid Glass | 2.168ms | 13.674ms | 0.00% | 187.71MiB |

Evidence is in `target/material-verification/gpu-final-solid-656ada827dd34c92af292762db2f7779`, `gpu-final-live-glass-826b9f534a774b189a7dfc2c8a437642`, and `gpu-final-liquid-glass-0b9b0d7a4dd74a8ba9995b7060adb528`. All three finite probes completed with the transition timer inactive. These measurements are not GPU time, p95 submission time, capture age or displayed FPS; a rounded zero CPU sample does not prove zero ongoing work.

## Remaining plan work and verification limits

- WGL_NV_DX_interop2 shared D3D11 textures and matching capture/OpenGL adapters are **not implemented**. All live frames use staging plus upload.
- Capture ROI does not yet include the requested extra sampling margin. Snapshot fallback is not yet downsampled on its worker.
- GPU query timings, capture/transport timing, frame age, per-frame allocations, displayed presentation cadence, and p95 latency targets are not measured. CPU rendering-notifier durations are not displayed FPS or GPU timings.
- An initial release Solid/Live Glass/Liquid Glass comparison was recorded with a compiler running concurrently: mean CPU renderer callback 1.912 / 2.522 / 1.615ms, respectively. Evidence directories are `target/material-verification/gpu-release-solid-7f017ffce54e486883999ee6da562bed`, `gpu-release-live-glass-f1b5246426d9481ba864c2056194d18c`, and `gpu-release-liquid-glass-3f14c41c8b204dc085d46b9c024425f8`. These are preliminary, not a latency acceptance result. A running user instance and changing desktop also affect the scene.
- No physical mixed-DPI, cross-adapter, suspend/lock, forced device-loss or unsupported-adapter matrix has been completed. Slot ownership/generation and device-loss tests need expansion.
- The settings preview shares the shader, but its rendered output has not yet been visually reviewed.
- Secondary text remains weak on the enlarged patterned media sample. Readability on arbitrary mixed backgrounds is not accepted; local contrast support needs further work within the requested opacity limit.
- No claim is made that lag is fixed, that settled live capture has zero redraws on every driver, or that the 4ms submission / 75ms age targets are met.

Run the owned material fixture with `scripts/test-liquid-gpu.ps1`. `scripts/measure-material.ps1 -Isolated` permits a finite release probe alongside an existing user instance using a separate profile and log directory.
