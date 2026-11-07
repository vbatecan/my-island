[CmdletBinding()]
param([Parameter(Mandatory)][string]$Version,[Parameter(Mandatory)][string]$MsixVersion,[switch]$Signed)
$ErrorActionPreference='Stop'
$workspace=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$expected=& (Join-Path $PSScriptRoot 'release-version.ps1') -Tag "v$Version"
if($expected.msix_version -ne $MsixVersion){throw 'MSIX version does not match Cargo version'}
$output=Join-Path $workspace 'target/release-assets'
New-Item -ItemType Directory -Force -Path $output | Out-Null
$exe=Join-Path $workspace 'target/msix-build/x86_64-pc-windows-msvc/release/still-island.exe'
$setup=Join-Path $workspace "target/dist/StillIsland-$Version-windows-x64-setup.exe"
$msix=Join-Path $workspace "target/msix/StillIsland_${MsixVersion}_x64_unsigned.msix"
foreach($path in @($exe,$setup,$msix)){if(-not(Test-Path -LiteralPath $path -PathType Leaf)){throw "Missing release artifact: $path"}}
$zip=Join-Path $output "StillIsland-$Version-windows-x64-portable.zip"
Compress-Archive -LiteralPath @($exe,(Join-Path $workspace 'LICENSE'),(Join-Path $workspace 'README.md')) -DestinationPath $zip -Force
$setupOutput=Join-Path $output ([IO.Path]::GetFileName($setup));Copy-Item -LiteralPath $setup -Destination $setupOutput -Force
$suffix=if($Signed){''}else{'-unsigned'}
$msixOutput=Join-Path $output "StillIsland-$Version-windows-x64$suffix.msix";Copy-Item -LiteralPath $msix -Destination $msixOutput -Force
$assets=@($zip,$setupOutput,$msixOutput)
$files=@($assets|ForEach-Object {[ordered]@{name=[IO.Path]::GetFileName($_);sha256=(Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant();bytes=(Get-Item -LiteralPath $_).Length}})
[ordered]@{version=$Version;msix_version=$MsixVersion;platform='windows';architecture='x64';signed=[bool]$Signed;files=$files}|ConvertTo-Json -Depth 5|Set-Content -LiteralPath (Join-Path $output 'release-manifest.json') -Encoding utf8
$checksums=@($files|ForEach-Object {"$($_.sha256)  $($_.name)"})
$checksums+="$((Get-FileHash -LiteralPath (Join-Path $output 'release-manifest.json') -Algorithm SHA256).Hash.ToLowerInvariant())  release-manifest.json"
$checksums|Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS') -Encoding ascii
$signing=if($Signed){'Windows artifacts are Authenticode signed. MSIX still requires normal Windows installation and notification consent.'}else{'Artifacts are unsigned. Windows may display an unknown-publisher/SmartScreen warning. The unsigned MSIX requires signing and trusted installation before it can provide notification access.'}
@"
Windows x64 release of Still Island $Version.

- **Setup EXE:** per-user Windows 11 installer; includes uninstall support. No administrator privileges or automatic startup.
- **Portable ZIP:** extract and run still-island.exe.
- **MSIX:** identity-bearing package for Windows notification support after installation and explicit consent.

$signing

The Setup EXE and portable ZIP do not give the application package identity; notification listening is intentionally unavailable there. Other overlay/media/audio features remain available. Do not run portable and MSIX instances at the same time.

Verify downloads with SHA256SUMS. Linux builds are not part of this release.
"@|Set-Content -LiteralPath (Join-Path $output 'release-notes.md') -Encoding utf8
Get-ChildItem -LiteralPath $output -File
