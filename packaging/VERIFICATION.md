# Notification/package verification

Performed without installing a package, modifying certificate trust, or requesting notification permission:

- Notification service compiled independently against windows 0.62.2.
- Four pure unit tests passed: bounded/control-free text, source-ID validation, icon-independent diffing, and a 2 MiB maximum app-icon cache.
- Portable runtime probe passed: UnavailablePortable remained explicit, no notification-content events occurred, no consent was requested, and no source app was activated.
- Windows SDK MakeAppx 10.0.26100.9457 accepted the x64 full-trust manifest, `uap3:userNotificationListener` capability, application entry, and all three original PNG assets.
- MakeAppx unpack/block-map validation passed after ZIP64 timestamp normalization.
- Two validation-only packages from the same developer executable had identical SHA-256 `07F0F4AFCDA9D71D83443D256E8120778FFAEF58253FD61347577563D40D547B`.
- These temporary validation artifacts deliberately used the current dynamic-CRT developer executable and are not distribution artifacts. Normal package builds use an isolated static-CRT Cargo target and audit imports before packaging.

## Final integrated artifact

- Default `scripts/package.ps1` static-CRT build completed successfully in 3m 58s.
- Final unsigned artifact: `target/msix/StillIsland_0.2.0.0_x64_unsigned.msix` (5,896,189 bytes).
- Final package SHA-256: `65AB5A586D9A4D6D0E34965D004AEA7E977239C0896C659BE76FD119F2293AA1`.
- Static executable: 11,157,504 bytes; SHA-256 `873C5F60493E0DE3E7EA3BD4BB0BB714121D8628F4B5F88635556D835A70EB51`.
- Executable audit lists 23 native Windows DLL imports and no VCRUNTIME/MSVCP/CONCRT or external GUI/browser runtime imports. Windows system components and a working graphics driver remain required.
- Final MakeAppx schema/content packing and normalized archive unpack/block-map validation passed.
- Repackaging the same final binary with `-SkipBuild` produced the identical final package hash. Proof: `packaging/final-repeatability.json`; audit: `target/msix/package-audit.json`.
- Source/Cargo/UI/build fingerprint `3e6f4c0281bc6ca9038d7950e0c07b129bba1b48e46f5b9d3ea9229bd25bb2e4` remained unchanged across the final build. Individual hashes are in `packaging/final-source-freeze.json`.
- The final MakeAppx-extracted executable ran its renderer/settings verification successfully and exited with code 0. A read-only query of that child's `GetPackageFullName` returned `APPMODEL_ERROR_NO_PACKAGE` (15700), as expected outside installation. Its notification state therefore remains UnavailablePortable. Actual renderer/settings captures and the stopped animation scheduler were checked. Audit: `target/msix/extracted-runtime-audit.json`.
- The extracted test used `--verify-settings`, which disables live notification collection regardless of saved preferences. No real notification bodies were captured. No other application was stopped; the test waited for the normal developer benchmark instance to exit before using the singleton.

Pending: deliberate signing/trust/installation by the user, followed by packaged notification consent/delivery/revocation testing. **No installed/identity-bearing notification-runtime success is claimed from these portable and packaging checks.** No package was installed/registered, no certificate was created/imported/trusted, no signing occurred, and no notification permission was requested.
