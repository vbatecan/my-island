# Finite, isolated material/morph probe. Sends no global keyboard or mouse input.
param([string]$Executable='target/release/still-island.exe',[ValidateSet('solid','acrylic','glass','live-glass')][string]$Appearance='glass',[string]$Label='current',[ValidateSet('default','hardware')][string]$CaptureDriver='default')
$ErrorActionPreference='Stop'
$workspace=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if(Get-Process still-island -ErrorAction SilentlyContinue){throw 'Exit the running island first; this probe does not stop user processes.'}
$binary=(Resolve-Path -LiteralPath $Executable).Path
$run=Join-Path $workspace ('target/material-verification/'+$Label+'-'+$Appearance+'-'+[guid]::NewGuid().ToString('N'))
$profile=Join-Path $run 'StillIsland'
New-Item -ItemType Directory -Path $profile -Force | Out-Null
@'
island_scale=1.3
notifications_enabled=false
online_lyrics=false
battery_previews=false
auto_volume=false
compact_now_playing=false
fullscreen_hide=false
launcher_enabled=false
'@ | Set-Content -LiteralPath (Join-Path $profile 'settings.ini')
$log=Join-Path $workspace 'verification.log'
$offset=if(Test-Path $log){@(Get-Content $log).Count}else{0}
$start=[Diagnostics.ProcessStartInfo]::new($binary)
$start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
$start.WorkingDirectory=$workspace;$start.Arguments='--verify --demo --appearance='+$Appearance
if($CaptureDriver -eq 'hardware'){$start.Arguments+=' --capture-hardware'}
$start.EnvironmentVariables['LOCALAPPDATA']=$run
$application=[Diagnostics.Process]::Start($start)
$clock=[Diagnostics.Stopwatch]::StartNew();$samples=[Collections.Generic.List[object]]::new()
try{
 while(-not $application.HasExited){
  if($clock.Elapsed.TotalSeconds -gt 30){throw 'Material probe exceeded 30 seconds.'}
  $application.Refresh();$t=$clock.Elapsed.TotalSeconds
  $phase=if($t -lt 3){'idle'}elseif($t -lt 6){'expanded'}elseif($t -lt 9){'media'}elseif($t -lt 12){'volume'}elseif($t -lt 14){'interruptions'}else{'settled'}
  $samples.Add([pscustomobject]@{Seconds=$t;Phase=$phase;PrivateBytes=$application.PrivateMemorySize64;WorkingSetBytes=$application.WorkingSet64;CpuSeconds=$application.TotalProcessorTime.TotalSeconds})
  Start-Sleep -Milliseconds 100
 }
 $samples|Export-Csv -NoTypeInformation -LiteralPath (Join-Path $run 'samples.csv')
 $lines=@(Get-Content $log|Select-Object -Skip $offset);$lines|Set-Content -LiteralPath (Join-Path $run 'verification.txt')
 $summary=foreach($phase in @('idle','expanded','media','volume','interruptions','settled')){
  $rows=@($samples|Where-Object Phase -eq $phase|Select-Object -Skip 8)
  if($rows.Count -ge 2){$first=$rows[0];$last=$rows[-1];[pscustomobject]@{Phase=$phase;PrivateMiB=[math]::Round(($rows.PrivateBytes|Measure-Object -Maximum).Maximum/1MB,2);WorkingSetMiB=[math]::Round(($rows.WorkingSetBytes|Measure-Object -Maximum).Maximum/1MB,2);CpuOneCorePercent=[math]::Round(100*($last.CpuSeconds-$first.CpuSeconds)/($last.Seconds-$first.Seconds),2)}}
 }
 $summary|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $run 'summary.json');$summary|Format-Table
 $lines|Where-Object {$_ -match 'render_metrics|complete frames='}|Write-Output
 Write-Output "Evidence: $run"
 if($application.ExitCode -ne 0 -or $lines -match 'FAILED' -or -not ($lines -match 'complete frames=\d+ active=false')){throw 'Material runtime checks failed; inspect evidence.'}
}finally{
 if(-not $application.HasExited){$null=$application.CloseMainWindow();if(-not $application.WaitForExit(2000)){$application.Kill()}}
 $application.Dispose()
}
