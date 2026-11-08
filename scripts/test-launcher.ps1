# Finite isolated native Slint regression. Launches only the owned marker fixture.
param([ValidateSet('debug','release')][string]$Configuration='release',[string]$Fixture='')
$ErrorActionPreference='Stop'
$workspace=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if(Get-Process still-island -ErrorAction SilentlyContinue){throw 'Exit the running island first; this test never stops another instance.'}
if(-not $Fixture){
    & (Join-Path $PSScriptRoot 'test-launcher-native.ps1') -Configuration $Configuration
    $Fixture=(Get-ChildItem (Join-Path $workspace 'target/launcher-native') -Directory | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName+'\fixture.json'
}
$owned=Get-Content -LiteralPath $Fixture -Raw | ConvertFrom-Json
if(Test-Path -LiteralPath $owned.gui_marker){throw 'Use a fresh unlaunched GUI fixture.'}
$run=Join-Path $workspace ('target\launcher-ui\'+[guid]::NewGuid().ToString('N'))
$profile=Join-Path $run 'StillIsland'
New-Item -ItemType Directory -Path $profile | Out-Null
@'
appearance=solid
notifications_enabled=false
online_lyrics=false
battery_previews=false
fullscreen_hide=false
launcher_enabled=false
'@ | Set-Content (Join-Path $profile 'settings.ini')
$trace=Join-Path $workspace 'verification.log'
$offset=if(Test-Path -LiteralPath $trace){@(Get-Content $trace).Count}else{0}
$start=[Diagnostics.ProcessStartInfo]::new((Join-Path $workspace "target/$Configuration/still-island.exe"))
$start.UseShellExecute=$false; $start.CreateNoWindow=$true; $start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
$start.WorkingDirectory=$workspace
$start.Arguments='--verify-launcher --verify-launcher-target="'+$owned.gui_target+'"'
$start.EnvironmentVariables['LOCALAPPDATA']=$run
$application=[Diagnostics.Process]::Start($start)
try{
    Start-Sleep -Milliseconds 1300
    $application.Refresh()
    $idle=[pscustomobject]@{State='idle';PrivateBytes=$application.PrivateMemorySize64;WorkingSetBytes=$application.WorkingSet64}
    Start-Sleep -Milliseconds 1900
    $application.Refresh()
    $expanded=[pscustomobject]@{State='launcher';PrivateBytes=$application.PrivateMemorySize64;WorkingSetBytes=$application.WorkingSet64}
    @($idle,$expanded)|Export-Csv -NoTypeInformation (Join-Path $run 'memory.csv')
    if(-not $application.WaitForExit(20000)){throw 'Launcher verification exceeded its finite deadline.'}
    if($application.ExitCode -ne 0){throw "Launcher exited $($application.ExitCode)."}
    $lines=@(Get-Content $trace | Select-Object -Skip $offset)
    $lines|Set-Content (Join-Path $run 'verification.txt')
    $lines|Where-Object {$_ -match '^launcher (PASS|FAILED)'}|Write-Output
    if($lines -match '^launcher FAILED'){throw 'Launcher assertions failed.'}
    if(-not ($lines -match '^launcher verification complete$')){throw 'Missing launcher completion evidence.'}
    if(-not (Test-Path -LiteralPath $owned.gui_marker)){throw 'Owned launch fixture did not execute.'}
    $history=Get-Content (Join-Path $profile 'launcher-recent.json') -Raw|ConvertFrom-Json
    if($history.ids[0] -ne 'fixture-launch'){throw 'Successful launch was not persisted.'}
    Write-Output "PASS launcher UI, owned launch, and history; $run"
    @($idle,$expanded)|Format-Table
}finally{
    if(-not $application.HasExited){$null=$application.CloseMainWindow();if(-not $application.WaitForExit(2000)){$application.Kill()}}
    $application.Dispose()
}
