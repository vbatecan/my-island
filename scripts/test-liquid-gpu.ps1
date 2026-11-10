param([string]$Executable='target/release/still-island.exe')
$ErrorActionPreference='Stop'
$workspace=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$binary=(Resolve-Path -LiteralPath $Executable).Path
$run=Join-Path $workspace ('target/liquid-gpu-verification/'+[guid]::NewGuid().ToString('N'))
$profile=Join-Path $run 'StillIsland'
New-Item -ItemType Directory -Path $profile -Force | Out-Null
@'
appearance=liquid-glass
notifications_enabled=false
online_lyrics=false
battery_previews=false
auto_volume=false
compact_now_playing=false
fullscreen_hide=false
launcher_enabled=false
shortcut_enabled=false
outside_click=false
'@ | Set-Content -LiteralPath (Join-Path $profile 'settings.ini')
$start=[Diagnostics.ProcessStartInfo]::new($binary)
$start.UseShellExecute=$false
$start.CreateNoWindow=$true
$start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
$start.WorkingDirectory=$run
$start.Arguments='--verify-liquid --appearance=liquid-glass --isolated-material-probe'
$start.EnvironmentVariables['LOCALAPPDATA']=$run
$application=[Diagnostics.Process]::Start($start)
try {
 if(-not $application.WaitForExit(30000)){throw 'Liquid fixture exceeded 30 seconds.'}
 if($application.ExitCode -ne 0){throw "Liquid fixture exited with $($application.ExitCode). Evidence: $run"}
 $lines=Get-Content -LiteralPath (Join-Path $run 'verification.log')
 $lines|Write-Output
 if(($lines -match 'FAILED|GPU setup failed|GPU rendering failed') -or -not ($lines -match 'liquid verification complete')){throw 'Liquid fixture failed.'}
 Write-Output "Evidence: $run"
} finally {
 if(-not $application.HasExited){$application.Kill()}
 $application.Dispose()
}
