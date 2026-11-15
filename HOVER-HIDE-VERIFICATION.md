# Hide on hover

Enable **Hide on hover** under Interaction. The preference defaults to off.
Approaching a collapsed island retracts its bottom edge upward while narrowing
its shoulders, with a short translation into the selected display's top edge. Leave the
reveal strip, then hold the top edge directly above the island for 350 ms to
restore it. The revealed island remains usable through that pointer visit.
Leaving rearms hiding for the next approach. Panels, settings, launcher and
pressed pointer buttons protect interaction; fullscreen policy remains separate.

## Automated checks

- `cargo check`: passed.
- `cargo test --workspace`: passed; main application: 208 passed, 2 ignored.
- Added coverage for fresh edge approach, 350 ms hold boundary, cancellation on
  exit or pressed buttons, reset, session lock, and preference migration/persistence.
- Tests cover policy behavior, not the live Win32 input handler or rendered slide.

## Upward morph regression

- `cargo check` and the six visibility tests passed after the animation change.
- An isolated `--verify-hover-morph --isolated-material-probe` run exercised the
  shared pointer-entry hide handler for notch and floating forms. All assertions
  passed: width and height contract, translation is upward and shorter than the
  original height, hidden motion settles and stops, reveal restores the shell
  and protects interaction, and motion-disabled hide/reveal apply immediately.
- Inspected renderer captures of the baseline, hiding and revealing states in
  `target/hover-morph-probe/verification`. Native window diagnostics accompanied
  the captures. This verifies shell rendering and animation orchestration; it
  does not test the physical edge-pressure gesture or glass appearance.

## Remaining manual coverage

## GPU Acrylic and native update optimization

Acrylic's per-frame contour masking and texture sampling now run through the
existing context-owned GPU renderer. Captures still prepare a bounded blurred
backdrop on arrival; changing the contour no longer regenerates CPU pixels.
The existing CPU compositor remains the fallback. Shape and translation share
one native submission, monitor/DPI queries are cached until display/configuration
changes, unchanged physical placement/region updates are skipped, and ordinary
motion no longer requests a nonclient frame recalculation. Eco reveal reuses its
material without a synchronous DWM affinity flush.

Validation: `cargo check`, workspace tests (209 application tests passed, three
ignored), owned Acrylic GPU/forced-CPU/Solid morph fixtures and Liquid Glass
color/tint regression fixture passed. Acrylic captures were inspected.

Matched release fixture measurements with speed 0.7, without screenshots:

| Path | p95 update work | p95 renderer interval | Mean renderer work |
| --- | ---: | ---: | ---: |
| GPU Acrylic | 5.871 ms | 11.037 ms | 1.279 ms |
| Forced CPU Acrylic | 6.733 ms | 11.026 ms | 0.718 ms |

Final-release smoke run after the material-profile publishing fix: GPU Acrylic
p95 update 5.476 ms, p95 renderer interval 11.172 ms, mean renderer work 1.153 ms.
Both notch/floating GPU assertions and motion-disabled checks passed. The
optimized release replaced the running debug instance; its previous executable
is retained as `target/hover-cpu-fallback.exe`.

Evidence is under `target/hover-performance/`; use
`scripts/test-hover-morph.ps1 -Mode gpu` (or cpu, visual, solid) to repeat.
These are single-run whole-fixture measurements, not isolated hide/reveal phase
statistics or displayed FPS. Both paths include the native optimizations; this
does not measure the complete pre-change implementation. No quantified
debug-to-release comparison is claimed. GPU work lowers CPU update cost here
but does not improve renderer intervals in this small fixture. The proposed
4 ms p95 update target remains unmet; native updates remain a likely contributor.
Physical gesture latency, display-cache invalidation across real monitor changes,
multi-monitor clipping and driver/context-failure recovery still need hands-on
verification. No global input was injected.

No global input was injected. The rebuilt debug instance was started after the
owned fixture completed. Physical edge-pressure and desktop click-through
verification remain manual; renderer captures alone do not establish those.

Check both notch and floating forms, compact activities, open panels, launcher,
dragging, rapid entry/exit, interrupted slides, keyboard reveal, and disabling
the preference while hidden. Check reduced motion and enlarged text. Confirm
the exposed area accepts clicks and that the island stays usable after reveal.
Check mixed DPI, negative monitor origins and a monitor above the selected
display: the moving silhouette must remain clipped to its selected monitor.
Inspect glass materials during movement and after reveal, and confirm timers
and capture stop once hidden. Use an owned fixture or manual interaction with
the island; do not send test input to other applications.
