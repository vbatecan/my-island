# Still Island plugin SDK v1

Plugins are trusted external Windows programs. The host never loads plugin DLLs, compiles plugin UI, or embeds a language runtime. A Rust SDK is provided; any language can implement the JSON-lines contract. Non-Rust authors must provide their own runtime/launcher as a packaged Windows executable.

## First plugin

1. Create a Rust binary and depend on `still-island-plugin-sdk` (path dependency in this workspace).
2. Add a manifest using `examples/developer/manifest.json` as the reference. Declare every action, view, configuration field, and packaged image.
3. Read `HostMessage` values with `run`, acknowledge Init with `Ready { protocol_version: PROTOCOL_VERSION }`, and publish `PluginMessage::View` with a monotonically increasing revision.
4. Represent the view as a `Node` tree. Containers are Row, Column, and List. Leaves are Text, Image, Button, Progress, Toggle, and Input. Every node has a unique ID. Interactive leaves reference a declared action.
5. Handle `Invoke { action_id, value }`. Inputs carry strings and toggles carry booleans. Respond using ActionResult, optionally requesting an explicit host URI action. Use `failure(action_id, message)` for recoverable errors so the host clears its action deadline. The SDK also converts an Invoke handler error into a failed ActionResult and continues; errors during initialization remain fatal.
6. Build a Windows release executable. Put `manifest.json`, the declared executable, and packaged assets at the root of a ZIP (preserving declared relative asset paths).
7. Import through Settings → Plugins, review the executable/capabilities, configure it, and explicitly enable it. Import alone does not run code.

All sample sources live in [examples/](../../examples/README.md). The developer sample demonstrates inputs, toggles, progress, row/column composition, action results, and proactive previews. Obsidian demonstrates local actions; GitHub Actions demonstrates optional authenticated networking; Codex Usage demonstrates a bounded, cancelable local app-server client with on-demand quota snapshots.

## Wire protocol

UTF-8, one JSON object per line. stdout is protocol-only; use stderr for bounded diagnostic messages without secrets. Each message is limited to 64 KiB including its newline. The first host message is `init` with protocol_version1 and an object of configured values. The plugin acknowledges with `ready` before publishing views.

Host messages: init, view_request, invoke, configure, shutdown. Plugin messages: ready, view, activity, action_result, error. Exact field names and Serde definitions are in `../plugin-protocol/src/lib.rs`; these are the normative version1 contract. Unknown extra fields are ignored for additive compatibility; unknown message/widget types and incompatible major versions are rejected.

An example update:

```json
{"type":"view","view_id":"main","revision":1,"root":{"id":"root","kind":"column","spacing":6,"children":[{"id":"title","kind":"text","text":"Hello"},{"id":"refresh","kind":"button","text":"Refresh","action":"refresh"}]}}
```

Layouts allow up to128 nodes and depth8. Padding/spacing are 0–32 logical pixels, flex is 0–100, progress is normalized0–1, and all numbers must be finite. Text is limited to4096 bytes per node. Images must reference a declared relative packaged asset. Layouts contain no scripts, shaders, font loaders, or absolute coordinates. The host owns clipping, focus, typography, accessibility, morphs, and frame scheduling.

Views may include optional `compact` metadata for the contracted notch: `{"title":"Codex · 63% used","body":"7-day limit · 37% left","progress":0.63,"accent":"green"}`. The title must contain text and fit128 UTF-8 bytes; the body defaults empty and fits256 bytes. Progress is optional, finite, and normalized0–1; accent defaults neutral. Rust authors use `CompactSummary` and `PluginMessage::View { compact: Some(summary), .. }`, or `compact: None` to omit it. Existing JSON plugins can continue omitting the field. This is persistent presentation metadata, separate from transient activities; label cached or stale information accurately.

## Activities and configuration

`activity` requests a brief native preview using id/title/body/accent. This is a request: the host may coalesce, defer, suppress, throttle, or expire it. Do not continuously emit activities or send animation frames; publish meaningful state changes. Accent values are neutral, green, amber, red, and blue.

Configuration fields are string, secret, bool, integer, number, or choice. Secrets arrive only in Init/Configure; never copy them into views, activity text, errors, or diagnostics. Declared capabilities describe behavior; trusted external programs still have their user's OS access and are not sandboxed.

Use `action_result.open_uri` only in response to the user's explicit action. The host decides which URI schemes are permitted. Do not automatically open applications in response to a background event.

## Reference integrations

- [Codex Usage](../../examples/codex-usage/README.md): uses an installed, ChatGPT-signed-in Codex executable to read reported quota windows through `account/rateLimits/read`. It supports explicit refresh/notch previews without background polling or inference. A custom executable path is optional.

- Obsidian: set `vault` to an existing vault name. Daily note requires Obsidian's Daily Notes feature. The plugin only returns encoded URI requests; the host executes explicit actions.
- GitHub Actions: set `repository` to owner/name. Optional token should have read-only Actions access for the selected repository. No existing GitHub CLI credentials are reused. `monitor` is opt-in. Active runs refresh at30s; otherwise300s, with conditional requests and rate-limit backoff. Startup is a silent baseline. Public repositories can be read without a token. Tokens are never embedded in URLs.

## Developer checks and packages

```powershell
cargo test --workspace --exclude still-island
cargo build --workspace --exclude still-island
python scripts/plugin-smoke.py
powershell -File scripts/package-plugins.ps1
```

`plugin-smoke.py --github` additionally reads a public GitHub workflow API. It never opens returned URLs or Obsidian, and does not use a user's token. The smoke fixture is a development tool, not a runtime dependency. Separate plugin ZIPs and checksums are generated under ignored `target/plugin-packages/`; CI uploads them independently from the application installer.

## Verify and import a reference package

Download a plugin ZIP and its accompanying SHA256SUMS artifact. Run `Get-FileHash -Algorithm SHA256 -LiteralPath <plugin.zip>` and compare the result with the matching filename in SHA256SUMS. Then choose Settings → Plugins → Import, select the ZIP, inspect its program/capabilities, configure required fields, and explicitly enable it. A checksum detects accidental artifact changes; it is not publisher authentication. Do not execute a ZIP entry directly as part of verification.

Release package scripts build the three examples with an explicit x86_64-pc-windows-msvc target and static CRT in isolated `target/plugin-build/`. The packager audits PE imports and rejects VCRUNTIME/MSVCP/CONCRT/MSVCR redistribution dependencies; it never packages the ordinary developer `target/release` outputs. `-SkipBuild` expects those isolated static outputs. Smoke the exact distribution binaries with `python scripts/plugin-smoke.py --binary-dir target/plugin-build/x86_64-pc-windows-msvc/release` (optionally `--github` for a live public API read). `-KeepStaging` retains ignored build-stage folders when local cleanup is unavailable.
