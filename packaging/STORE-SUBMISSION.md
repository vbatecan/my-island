# Microsoft Store submission

Still Island is registered as an MSIX app, Store ID **9ND16NLPK4SQ**. The public identity is recorded in `store-identity.json`. Do not substitute the development identity or copy its publisher into a Store package.

## Build

Run `pwsh -NoProfile -File scripts/package.ps1 -Store`. This builds the static-CRT Windows x64 release and generates `target/store/StillIsland_<version>_x64_store.msix`, with its audit report. Package identity is Nytri.StillIsland, publisher CN=CA9B83B5-27E1-4295-BC3C-82297BA1DC6E, publisher display name Nytri. Store package versions consistently offset the Cargo major by one and append `.0`: Cargo 0.3.1 becomes Store 1.3.1.0, and Cargo 1.0.0 becomes Store 2.0.0.0. This keeps Store versions increasing and satisfies its nonzero-major rule. The fourth component stays zero for Store submission.

The Microsoft Store package workflow provides the same artifact for a selected commit or tag. Upload the MSIX itself from the downloaded artifact to Partner Center; the Actions artifact ZIP is not the submission package. No certificate secrets are needed for this build. Microsoft signs accepted Store packages. Sideloaded test installation has separate signing/trust requirements; the script does not install or trust anything.

Store-signed installations use Windows Store updates and expose an Open Microsoft Store action. They do not stage or execute the GitHub updater helpers. GitHub installer and portable distributions retain their updater. Development/sideloaded MSIX builds retain their existing manual release link.

## Listing draft

**Description:** Still Island brings a quiet, responsive island to the top of your Windows desktop. Control supported media players and system volume, set a countdown, launch applications, and calculate or convert values without opening a large window. Customize its size, appearance, motion, and monitor placement. Optional notification previews require Windows permission.

**Features:** Media playback controls for compatible players; system volume and mute; countdown timer; keyboard application launcher; calculator and currency conversion; optional notification previews; adjustable appearance, text size, and motion.

Use Windows 11 build 22621 or newer on x64. Do not advertise native ARM64 support. Complete pricing, markets, category, age rating, support contact, and a public privacy-policy URL in Partner Center. Supply real screenshots of the release using nonprivate fixture content. Review the generated listing text against the tested release before submission.

## Certification notes draft

This is a native Win32 desktop utility packaged as MSIX. `runFullTrust` is required for its existing Win32 desktop overlay, tray, global shortcuts, media/audio integration, and application launcher. It does not request administrator elevation. Startup is opt-in through the declared StartupTask.

`userNotificationListener` supports optional notification previews. Open tray Settings → Notifications and choose Enable access; Windows prompts for consent. Denial leaves other features usable. Notification previews do not dismiss Windows Notification Center records.

Online lyrics, currency conversion, and user-installed external integrations are optional network/process features. No external integrations are automatically installed or enabled. Include their actual behavior in privacy disclosures and review it against current Store policies; these notes do not establish certification approval.

## Submission checks

Run the Windows App Certification Kit against the final package. Test the Store identity using an appropriately signed test package or Partner Center flight: first launch, notification grant/denial, startup opt-in, uninstall, and upgrade from a preceding Store package. Confirm packaged copies open Microsoft Store rather than GitHub for updates. Review enlarged-text layout and screenshots. Local MakeAppx validation is not Store certification.

In Partner Center, create the submission, upload the MSIX, complete the listing and declarations, review certification notes, and choose publication timing. A flight or manual publication hold is appropriate for the first release. The repository workflow only produces an artifact; it does not submit or publish the Store product.

References: [MSIX package requirements](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements), [Store URI links](https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-store-app), [reserved product](https://apps.microsoft.com/detail/9ND16NLPK4SQ).
