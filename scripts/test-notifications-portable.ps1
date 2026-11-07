# Compiles/runs only the portable capability-gate probe and pure parser/cache tests.
# It never installs/signs an MSIX, asks for notification permission, or posts toasts.
$ErrorActionPreference='Stop'
$workspace=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$source=(Join-Path $PSScriptRoot 'test-notifications-portable.rs').Replace('\','/')
$service=(Join-Path $workspace 'src/services/notifications.rs').Replace('\','/')
$taskDir=Join-Path ([IO.Path]::GetTempPath()) ('StillIsland.NotificationsProbe.'+[Guid]::NewGuid().ToString('N'))
$null=New-Item -ItemType Directory -Force $taskDir
@"
[package]
name="still-notifications-portable-probe"
version="0.1.0"
edition="2024"
[dependencies]
windows={version="=0.62.2",features=["Foundation","ApplicationModel","Storage_Streams","Security_Authorization_AppCapabilityAccess","UI_Notifications","UI_Notifications_Management","Win32_Foundation","Win32_Storage_Packaging_Appx","Win32_System_Com","Win32_System_WinRT","Win32_UI_Shell","Win32_UI_WindowsAndMessaging"]}
[lib]
path="lib.rs"
[[bin]]
name="portable-probe"
path="$source"
"@|Set-Content -LiteralPath (Join-Path $taskDir 'Cargo.toml')
@"
#[path="$service"]
pub mod notifications;
"@|Set-Content -LiteralPath (Join-Path $taskDir 'lib.rs')
$manifest=Join-Path $taskDir 'Cargo.toml'
$target=Join-Path $workspace 'target/notification-probe'
& cargo test --offline --manifest-path $manifest --target-dir $target --lib
if($LASTEXITCODE -ne 0){throw 'Notification parser/cache tests failed'}
& cargo run --offline --manifest-path $manifest --target-dir $target --bin portable-probe
if($LASTEXITCODE -ne 0){throw 'Portable notification safety probe failed'}
