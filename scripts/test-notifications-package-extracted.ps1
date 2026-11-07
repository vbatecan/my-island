param([string]$AuditPath)
# Runs only the extracted, unpackaged executable. It does not install/register the
# MSIX, sign/trust certificates, request notification access, or send real toasts.
$ErrorActionPreference='Stop'
$workspace=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if(-not $AuditPath){$AuditPath=Join-Path $workspace 'target/msix/package-audit.json'}
$audit=Get-Content -LiteralPath $AuditPath -Raw|ConvertFrom-Json
$executable=Join-Path $audit.UnpackValidation 'still-island.exe'
if(-not(Test-Path -LiteralPath $executable)){throw 'MakeAppx-validated extracted executable was not found'}
if((Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash -ne $audit.ExecutableSha256){throw 'Extracted executable differs from audited input'}
Add-Type @'
using System;
using System.Runtime.InteropServices;
namespace StillIslandPackageProbe {
  public static class Identity {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetPackageFullName(IntPtr process, ref uint length, IntPtr name);
  }
}
'@
$directory=Join-Path (Split-Path -Parent $AuditPath) ('extracted-runtime-'+[Guid]::NewGuid().ToString('N'))
$null=New-Item -ItemType Directory -Force $directory
$process=Start-Process -FilePath $executable -ArgumentList '--verify-settings','--appearance=solid','--demo' -WorkingDirectory $directory -WindowStyle Hidden -PassThru
try {
    Start-Sleep -Milliseconds 650
    if($process.HasExited){throw "Extracted application exited during startup ($($process.ExitCode))"}
    [uint32]$length=0
    $identity=[StillIslandPackageProbe.Identity]::GetPackageFullName($process.Handle,[ref]$length,[IntPtr]::Zero)
    if($identity -ne 15700){throw "Expected APPMODEL_ERROR_NO_PACKAGE (15700), received $identity. This must remain an unpackaged probe."}
    if(-not $process.WaitForExit(25000)){throw 'Extracted verification did not complete within 25 seconds'}
    if($process.ExitCode -ne 0){throw "Extracted application failed ($($process.ExitCode))"}
    $log=Get-Content -LiteralPath (Join-Path $directory 'verification.log') -Raw
    if($log -match 'FAILED' -or $log -notmatch 'complete frames=\d+ active=false' -or $log -notmatch 'settings snapshot'){throw 'Extracted renderer/settings verification failed; inspect verification.log'}
    $result=[ordered]@{Executable=$executable;ExecutableSha256=$audit.ExecutableSha256;IdentityResult=$identity;Identity='UnavailablePortable';RuntimeDirectory=$directory;SettingsSnapshot=(Join-Path $directory 'verification/settings.ppm');ExitCode=$process.ExitCode;Installed=$false;PermissionRequested=$false}
    $result|ConvertTo-Json|Set-Content -LiteralPath (Join-Path (Split-Path -Parent $AuditPath) 'extracted-runtime-audit.json') -Encoding UTF8
    Write-Output 'PASS extracted static-CRT application: no package identity, renderer/settings verification complete, clean exit.'
    Write-Output "Runtime artifacts: $directory"
} finally {if(-not $process.HasExited){$null=$process.CloseMainWindow();if(-not $process.WaitForExit(2000)){Stop-Process -Id $process.Id}}}
