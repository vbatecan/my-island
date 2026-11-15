# Interaction, activities and launcher refinement

The conditional checkbox is under Hide on hover in Interaction. Its saved key
is `hover_only_maximized`, defaulting to false. With both options enabled,
ordinary windows keep the island visible, maximized windows permit hover hiding,
and fullscreen surfaces hide automatically. The saved fullscreen preference is
retained while its control displays the enforced state. Existing explicit app
overrides and session-lock handling continue to apply.

Keep visible is a session-only tray/launcher action. Panel pinning has its own
lifetime. Timer completion takes compact priority and remains pending through
hidden or pinned-panel states. A panel alert opens the timer to acknowledge it.
The compact completion receives five visible seconds before yielding.

The launcher accepts whole-second durations from one second through 24 hours,
supports explicit timer replacement, and revalidates query and timer revision at
activation. Percentage-of expressions and adjacent/symbol currency inputs use
the existing calculator and exchange-rate service. Slow currency lookups receive
one delayed indication. Settings now have Appearance, Interaction, Activities,
Visibility and General sections.

## Automated and rendered verification

- Workspace tests passed with 213 application tests passing and three opt-in
  checks ignored, plus the workspace plugin/protocol/SDK suites.
- The owned product fixture passed checkbox persistence wiring, all five settings
  section captures, timer creation, replacement, stale-action rejection, shared
  visibility hold commands, pinned-panel completion and acknowledgement.
- Initial capture review found a media-picker overlap and an incorrect panel
  heading during timer completion. Header space and title conditions were fixed.
- Enlarged text at 140% exposed completion-heading clipping. The heading now
  uses the full row, duration controls move below it, and the repeated completion
  subtitle is removed. The corrected capture was inspected.
- Optimized release product and GPU Acrylic hover fixtures passed. The latter
  confirmed GPU rendering for notch and floating forms, settled hidden motion,
  and motion-disabled hide/reveal. Evidence is under `target/product-verification`
  and `target/hover-performance`.
- `scripts/test-product.ps1` repeats the callback and renderer checks using an
  isolated profile. It does not inject keyboard or mouse input globally.

## Native verification limits

The scoped native fixture verified selected-monitor positioning, then reported
an inconclusive result because Windows denied foreground focus to its owned
window. A UI Automation focus attempt also failed. No input was injected to
force focus. Consequently the ordinary/maximized/fullscreen/restored native
transition assertions could not complete. Use
`scripts/test-fullscreen.ps1 -ScopedHover -SkipHover` on an interactive desktop
where the owned fixture can acquire focus.

Exclusive fullscreen games, mixed-DPI monitor changes and physical edge-pressure
latency require manual verification. The pure policy tests cover scope, lock,
hold, default migration and persistence. Renderer captures and callback fixtures
alone do not establish native gesture correctness.
