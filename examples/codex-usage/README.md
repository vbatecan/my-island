# Codex Usage

A native Still Island plugin that displays Codex's **reported quota usage**, not an estimate based on token counts. Available windows show used/remaining percentages, progress bars, and time until their reported resets. Window lengths come from the response; a missing window stays absent.

## Install

1. Install Codex and sign in using your ChatGPT account. Check that Codex itself can show usage limits.
2. From the repository root, run `powershell -File scripts/package-plugins.ps1`.
3. In **Still Island Settings → Plugins**, import `target/plugin-packages/org.stillisland.codex-usage-0.2.0-windows-x64.zip` and enable **Codex Usage**.
4. Open its **Usage** view. Use **Refresh** for a new reading, or **Show in notch** for a brief compact preview. You can pin that action in Integrations.

With the current island release, scroll the expanded notch to **Codex Usage**. Its large percentage metrics show each reported window, with remaining usage and reset information below. When the cursor leaves, the compact notch keeps a badge, the most constrained reported percentage, and a progress line. Hover again to reopen Codex. The page choice persists across restarts, and the selected integration can load a new snapshot once at startup; it still has no background polling loop.

The executable setting may be left blank: the plugin checks the standard Windows Codex installation, then `codex.exe` on PATH. Set an explicit path to the real Codex executable for a custom installation. The field is a filename/path, not a shell command; no arguments or quoting are needed. A recent Codex app-server interface is required; this was verified with Codex CLI 0.166.0.

## Data and lifecycle

The plugin starts a short-lived, hidden `codex app-server --listen stdio://` child using your existing Codex configuration/sign-in. It initializes the protocol and calls only `account/rateLimits/read`. It does not create a thread, start inference, read `auth.json`, accept API keys, or scrape private web endpoints. Codex handles its own authenticated account request. Existing parent Codex processes are untouched; cleanup terminates only this request's child.

Quota data is a snapshot. Opening the Usage view for the first time fetches it; subsequent views reuse it until **Refresh**. No background polling or per-frame updates are installed. **Show in notch** uses the last snapshot, or reads one first if necessary. Automatic resets are never fabricated locally: an elapsed reset asks for another refresh. API-key-only accounts, unsupported responses, unavailable quotas, and sign-in errors show an explanatory state. If a refresh fails after a successful reading, the previous values remain explicitly marked as last reported usage.

Concurrent refresh clicks share the in-flight request. The interface remains responsive while reading. Requests have an eight-second deadline and bounded output; disabling/stopping the plugin cancels and cleans up its worker. Analytics/trace exporters are disabled for the temporary Codex child. Remote error contents and raw account responses are not shown or logged.

`local_process:codex` and network capabilities describe the delegated account lookup. Like the other plugins, it is a trusted external program with the Windows user's access, rather than a sandboxed extension. Install/enable only packages you trust.

## Verify

```powershell
cargo test -p still-island-plugin-codex-usage --locked
cargo build --release -p still-island-plugin-codex-usage --locked
python scripts/test-codex-usage.py --binary-dir target/release
```

The process smoke test uses an offline synthetic app-server fixture. To perform an actual read-only check with your current Codex sign-in:

```powershell
target/release/still-island-plugin-codex-usage.exe --check
```

This prints only normalized quota windows and reset timestamps, or a sanitized error. It performs an account request; no inference is run. On this machine, the live response supplied a 7-day window at 22% used; no primary window was returned. That is a point-in-time verification result, not a preset or guaranteed account entitlement.

The method, multi-bucket handling, and percentage/reset semantics follow the [official Codex app-server documentation](https://learn.chatgpt.com/docs/app-server#6-rate-limits-chatgpt). The client prefers `rateLimitsByLimitId.codex`, with the documented legacy `rateLimits` fallback; unrelated model-specific buckets are not substituted.
