param([ValidateSet('debug','release')][string]$Configuration='release')
$ErrorActionPreference='Stop'
$workspace=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$executable=Join-Path $workspace "target\$Configuration\still-island.exe"
if(-not (Test-Path -LiteralPath $executable)){throw "Build the $Configuration executable first."}
if(Get-Process still-island -ErrorAction SilentlyContinue){throw 'Exit the running island before this test; it never stops another instance.'}
$profile=Join-Path $workspace ('target\polish-probe\'+[guid]::NewGuid().ToString('N'))
$settings=Join-Path $profile 'StillIsland'
New-Item -ItemType Directory -Force -Path $settings | Out-Null
@'
appearance=solid
form_factor=notch
text_scale=1
animations=true
animation_speed=1
follow_reduced_motion=false
compact_now_playing=false
auto_volume=false
notifications_enabled=false
online_lyrics=false
battery_previews=false
outside_click=false
shortcut_enabled=false
fullscreen_hide=false
'@ | Set-Content -LiteralPath (Join-Path $settings 'settings.ini')
$log=Join-Path $workspace 'verification.log'
$results=@()
foreach($test in @(
    @{Flag='--verify-interactions';Done='interaction verification complete'},
    @{Flag='--verify-layout';Done='layout verification complete'},
    @{Flag='--verify-illustrations';Done='illustration verification complete'},
    @{Flag='--verify-lyrics';Done='lyrics verification complete'},
    @{Flag='--verify-media-polish';Done='media polish verification complete'}
)){
    $offset=if(Test-Path -LiteralPath $log){@(Get-Content -LiteralPath $log).Count}else{0}
    $start=[Diagnostics.ProcessStartInfo]::new($executable)
    $start.UseShellExecute=$false
    $start.CreateNoWindow=$true
    $start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.WorkingDirectory=$workspace
    $start.Arguments=$test.Flag+' --appearance=solid'
    $start.EnvironmentVariables['LOCALAPPDATA']=$profile
    $application=[Diagnostics.Process]::Start($start)
    try{
        if(-not $application.WaitForExit(45000)){throw "Timed out: $($test.Flag)"}
        if($application.ExitCode -ne 0){throw "Exited with code $($application.ExitCode): $($test.Flag)"}
        $newLines=@(Get-Content -LiteralPath $log | Select-Object -Skip $offset)
        $newLines | Set-Content -LiteralPath (Join-Path $profile ($test.Flag.TrimStart('-')+'.log'))
        $failures=@($newLines | Where-Object {$_ -match 'FAILED|panicked'})
        if($failures.Count){throw ($failures -join [Environment]::NewLine)}
        if(-not @($newLines | Where-Object {$_.StartsWith($test.Done)}).Count){throw "Missing completion evidence: $($test.Flag)"}
        $results += [pscustomobject]@{Test=$test.Flag;Pass=$true;Checks=@($newLines|Where-Object{$_ -match ' PASS '}).Count}
        Write-Output "PASS $($test.Flag)"
    }finally{
        if(-not $application.HasExited){$application.Kill();$application.WaitForExit()}
        $application.Dispose()
    }
}
$results | Export-Csv -NoTypeInformation -LiteralPath (Join-Path $profile 'results.csv')
Write-Output "Isolated profile and test logs: $profile"
