# Windows releases

The private repository is [vbatecan/my-island](https://github.com/vbatecan/my-island). The workflows are `windows-ci.yml` (Windows checks/builds), `workflow-lint.yml` (workflow validation), and `windows-release.yml` (tagged Windows release artifacts). Review their results in GitHub Actions before distributing a build. These files define Windows x64 support; Linux packaging is deferred because the application currently uses Windows-only platform modules.

Keep `Cargo.toml` and `Cargo.lock` consistent, commit the intended release, and use a stable tag matching the package version. The initial version is **0.2.0**, so the matching tag is **v0.2.0** and the MSIX identity version is **0.2.0.0**. `scripts/release-version.ps1` validates the exact tag and reads only the package version; it never edits files or creates a tag. Prerelease/build suffixes, leading zeros, and version components exceeding 65535 are rejected. The workflow outputs are `version` and `msix_version`.

```powershell
.\scripts\release-version.ps1 -Tag v0.2.0
# After reviewing/committing the intended release and passing local checks:
git tag v0.2.0
git push origin v0.2.0
```

The tag workflow builds the native executable, portable archive, classic Setup installer, MSIX package, and checksums, then publishes the release artifacts. The classic installer/portable build works without MSIX identity, but **Windows notification-listener access still requires the identity-bearing MSIX installation and explicit Windows consent**. A classic installer does not confer that identity. Exit an existing portable instance before starting the installed MSIX application. Unsigned EXE/Setup artifacts may encounter normal Windows reputation prompts; an unsigned MSIX is clearly named unsigned and requires deliberate signing before normal installation. Neither CI nor the packaging scripts import certificate trust or install the package.

Signing is optional and uses exactly three GitHub secrets, configured together: `WINDOWS_CERTIFICATE_PFX` (base64 PFX containing the private key), `WINDOWS_CERTIFICATE_PASSWORD`, and `WINDOWS_SIGNING_PUBLISHER` (the certificate's exact Subject and MSIX manifest Publisher). All absent means an unsigned release; partial configuration fails rather than silently falling back. `scripts/sign-release.ps1 -Paths ... -ExpectedPublisher ...` validates private-key availability, current validity, exact publisher, and Code Signing EKU using an ephemeral certificate load. It writes `signing_enabled` when an output file is supplied; `-RequireSignature` rejects the unsigned mode.

Sign the self-contained executable **before** packing installers/MSIX, then sign the completed installers/packages and verify their signatures before checksums/upload. The helper uses SHA-256, a DigiCert RFC 3161 timestamp, SignTool's Authenticode verification policy, and signer/timestamp checks. It never prints signing arguments or secrets, removes the temporary PFX, and performs no certificate-store import or trust modification. Run signing only on isolated trusted release runners; do not expose these secrets to untrusted pull-request code. Modern hardware-backed/HSM signing services may require a future provider-specific adapter rather than an exportable PFX.

Local helper verification covered stable/version mismatch/range/input validation, unsigned no-mutation, required-signature failure, incomplete secrets, publisher mismatch, invalid base64, and invalid PFX. Successful CA-backed signing was not exercised locally without a signing certificate. A workflow definition/lint pass is not proof of a completed GitHub-hosted release; inspect the actual run and its uploaded artifacts. Signing/timestamps change bytes, so checksums must be generated after the final signatures.

Microsoft references: [SignTool options and verification](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool) and [MSIX certificate/publisher requirements](https://learn.microsoft.com/en-us/windows/win32/appxpkg/how-to-sign-a-package-using-signtool). See [README.md](README.md) for local MSIX building/signing/installation details.
