# MSIX packaging and notification identity

The portable executable remains usable without installation. Receiving Windows notifications through `UserNotificationListener.NotificationChanged` additionally requires a supported package identity, the declared capability, explicit Windows consent, and the in-app opt-in setting. The portable build reports **UnavailablePortable** and does not attempt a workaround.

`AppxManifest.xml` declares an x64, medium-integrity/full-trust desktop application with `uap3:userNotificationListener`, `rescap:runFullTrust`, and an opt-in desktop StartupTask (`StillIslandStartup`, initially disabled). It does not declare network, service, background-task, or administrator capabilities. The minimum platform is Windows 11 build 22621. The capsule logo assets are original, generated locally by `generate-assets.ps1`; no external asset download is required.

## Build an unsigned package

Requirements: Windows x64 Rust/MSVC toolchain, Windows SDK with x64 `MakeAppx.exe`, and the normal project's build prerequisites. The script discovers the newest installed SDK; it does not download or install tools.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1
```

The default build uses an isolated `target/msix-build` Cargo directory and `-C target-feature=+crt-static`. It leaves the normal release build intact and avoids an external Visual C++ runtime dependency in the package. The PE import audit rejects dynamically linked VC runtime DLLs and known external GUI/browser frameworks. Native Windows system DLLs and the installed graphics driver remain required. The audit covers the executable's direct imports; it is not an exhaustive driver/system dependency inventory.

Outputs:

- `target/msix/StillIsland_0.2.0.0_x64_unsigned.msix`
- `target/msix/package-audit.json`: executable/package SHA-256, imports, SDK, validation locations, and explicit unsigned/not-installed status.
- Fresh `stage-*` and `validation-*` directories. Prior artifacts are preserved; the script does not recursively delete staging directories.

After a successful build, repeat packaging without rebuilding:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1 -SkipBuild
```

`-Version 0.2.0.1` changes the four-part identity version. `-Publisher 'CN=YourSigningIdentity'` changes the manifest publisher; its exact distinguished name must match the signing certificate. `-Executable` is supported only with `-SkipBuild`, and its PE architecture must be x64.

The `-AllowDynamicCrt` switch exists solely for temporary packaging validation against a normal developer release. Such artifacts are explicitly reported as validation-only and must not be distributed as the self-contained build.

## Repeatability and validation

MakeAppx validates manifest schemas, asset references, and application entries while packing. The script normalizes unsigned ZIP/ZIP64 local/central header timestamps to 2026-01-01, without changing payloads, compressed streams, CRCs, block hashes, lengths, or offsets. It then runs MakeAppx unpack validation on the normalized archive. Identical executable/assets/manifest inputs produce identical unsigned package bytes on the tested SDK; a pair of independent validation packages was compared by SHA-256. Rebuilding Rust source is not claimed to be byte-identical across toolchains, machines, or changed build inputs. Signing introduces certificate/timestamp-dependent differences.

Regenerate logos only when intentionally changing the package artwork:

```powershell
powershell -NoProfile -File packaging/generate-assets.ps1
```

## Signing and installation are separate user actions

**The package is unsigned and is not directly installable through the usual App Installer flow. The packaging script never creates/imports certificates, signs, trusts, installs, or requests notification permission.**

For distribution, use your organization's approved code-signing certificate/process or Microsoft Store signing. For local testing, a developer may create their own code-signing certificate whose Subject matches the manifest Publisher and sign the resulting package with the Windows SDK SignTool:

```powershell
# Only after deliberately choosing/creating a certificate in CurrentUser\My:
# Supply your own certificate thumbprint. This command does not establish trust.
signtool sign /fd SHA256 /sha1 YOUR_CERTIFICATE_THUMBPRINT target/msix/StillIsland_0.2.0.0_x64_unsigned.msix
```

Review and trust a local test certificate only through your chosen explicit certificate-management process. Do not blindly trust downloaded test certificates. Once the signed package and signer are trusted, installing the package is a separate deliberate action:

```powershell
Add-AppxPackage -Path target/msix/StillIsland_0.2.0.0_x64_unsigned.msix
```

Exit any running portable instance from its tray first, then launch **Still Island** from Start after installation. The shared single-instance mutex does not give an already-running portable process a package identity; running the portable `target/release` executable still has no package identity. Enable notifications in the app and choose its permission action. Windows displays the consent prompt; denied permission can only be changed by the user in Windows Settings. The app does not automate that setting. Consent is requested on the foreground UI thread only after the user's action. Permission denial/revocation leaves other island functionality available.

The API opens a notification's registered source application through its AUMID when supported. It does not promise a deep link to a particular notification or replay toast activation arguments. Sources without a resolvable AppsFolder entry have no Open action.

## Safe checks without installing or consenting

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-notifications-portable.ps1
```

This compiles/runs parser/cache tests and verifies that an unpackaged process remains UnavailablePortable even after its local enabled flag is toggled. It does not request consent, enumerate notifications, send fixture toasts, launch applications, or install a package.

The final extracted self-contained binary can be checked without installation or consent:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-notifications-package-extracted.ps1
```

This verifies that the extracted process has no package identity, runs its safe renderer/settings harness, and exits cleanly. It does not validate the installed notification API branch. Exit any running island instance first so the single-instance guard does not intentionally reject the test.

Packaged notification delivery, permission denial/revocation, and real-provider event behavior must be tested after a user deliberately signs/installs the package and grants access. The implementation suppresses the initial existing baseline, checks consent before reading/emitting content, listens for capability-access changes, clears in-memory content on disable/revocation, and maintains a bounded diff/icon cache. Notification bodies are never written to disk or telemetry.

Primary references: [Microsoft notification listener guide](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/notification-listener), [capability declarations](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations), [manual MSIX generation](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion), and [Microsoft-tracked unpackaged event limitation](https://github.com/microsoft/WindowsAppSDK/issues/6172).
