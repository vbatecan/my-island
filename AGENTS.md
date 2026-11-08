# Repository instructions

## CodeGraph

When `.codegraph/` exists, use `codegraph explore "<symbols or question>"` or the available CodeGraph MCP tool before text search or file reads for code discovery. Do not reindex without the user deciding to do so. If no index exists, use ordinary repository search.

## Product and visual intent

Still Island is a restrained native Windows notch and launcher. The continuous black shape, readable content and fluid spring motion are the product. Preserve the existing visual language; do not invent a dashboard, instructional panel, status bar or decorative container for each feature.

The user explicitly rejected engine branding, repeated Enter/Copy labels, permanent arrow/Escape instructions, redundant footers, and duplicate “Calculating…” messages. Do not reintroduce these in another state or component. Keep keyboard functionality and accessible names without visible instruction clutter.

## UI decisions before implementation

- Start from the user's task and the existing component. Every new label, control, border and animation must have a distinct purpose.
- Use one hierarchy: primary result first; secondary information only when it changes interpretation or provides a useful next action. Currency rate dates and stale/offline status matter. Engine names, renderer details and debugging state do not belong in ordinary UI.
- Never repeat one state in a heading, body and footer. Loading title plus the same loading subtitle is a defect. Do not reserve space for explanations that are empty or unnecessary.
- Keep copy short and concrete. Map backend failures to actionable messages; never show raw parser errors, stack traces or diagnostic identifiers.
- Match established spacing, alignment, curvature, typography and contrast. Do not add borders, shadows or explanations just to fill empty space.
- When the user removes information, do not reintroduce it as a synonym, icon, tooltip or another explanatory block unless it serves a separate required purpose.

## Editing, loading and errors

Distinguish empty, incomplete input, pending work, successful result and actionable failure.

- `3+`, `sin(` and an unfinished conversion are normal typing states. They must not show a result card, loading message or error. Accept input quietly.
- Do not show progress for debounce or a quick local calculation. Show an answer when one exists, not an empty answer card.
- For genuinely slow operations, use at most one restrained, delayed indication, only when useful. Never duplicate loading text or create perpetual animation. Stop feedback on completion, cancellation and dismissal.
- Pending or invalid results cannot execute. Late responses cannot move selection, publish stale answers, overwrite newer queries or launch unrelated applications.
- Keep meaningful recovery information. Simplifying copy does not justify hiding stale rates or clipboard/network failures.

## Review and verification

Review changed states, not just the success screen. As relevant, inspect empty input, partial expressions, valid results, errors, slow currency lookup, long/enlarged text, keyboard selection and dismissal. Look for duplicate words, premature errors, clipping, jumpy geometry and unnecessary space.

Reuse native components and animation primitives. Separate text rasterization from shape scaling. Honor animation speed and reduced motion. Avoid continuous idle redraws and workers.

Build/check affected code and use focused behavioral tests for state changes. Inspect rendered output for layout changes: compilation is not visual approval. Never claim a visual fix from a probe sampling other windows or a test bypassing the runtime handler. Record verification limits honestly.

Do not inject global keyboard/mouse input while the user is using other applications. Prefer owned fixtures and component-local events. Test helpers must not send Escape or commands into another foreground application.

## Design references

- [Microsoft progress controls](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/progress-controls): some work needs no progress indicator; choose feedback according to user need.
- [Microsoft UI text](https://learn.microsoft.com/en-us/windows/win32/uxguide/text-ui): concise, understandable interface language.
- [Desktop UX checklist](https://learn.microsoft.com/en-us/windows/win32/uxguide/top-violations): useful feedback and consistent interaction.

The concrete examples above record this user's preferences, not universal rules for every interface. User instructions take precedence.
