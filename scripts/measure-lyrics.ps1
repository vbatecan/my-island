param(
    [ValidateRange(3,30)][int]$WarmupSeconds=8,
    [ValidateRange(3,30)][int]$SampleSeconds=8,
    [switch]$DisableOnlineLyrics
)
$ErrorActionPreference='Stop'
$workspace=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$executable=Join-Path $workspace 'target\release\still-island.exe'
if(-not (Test-Path -LiteralPath $executable)){throw 'Build the release executable first.'}
if(Get-Process still-island -ErrorAction SilentlyContinue){throw 'Exit the existing island before measuring; the benchmark never stops another instance.'}
# An isolated profile preserves normal preferences and disables automatic previews.
# Lyrics stay enabled; opening Media may look up the current track's metadata.
$profile=Join-Path $workspace 'target\lyrics-perf-profile'
$profileSettings=Join-Path $profile 'StillIsland'
New-Item -ItemType Directory -Force -Path $profileSettings | Out-Null
@'
appearance=solid
form_factor=notch
notch_corner_radius=8
animations=true
topmost=true
compact_now_playing=false
auto_volume=false
notifications_enabled=false
hover_expand=false
fullscreen_hide=false
outside_click=false
shortcut_enabled=false
online_lyrics=true
'@ | Set-Content -LiteralPath (Join-Path $profileSettings 'settings.ini')
if($DisableOnlineLyrics){
    $path=Join-Path $profileSettings 'settings.ini'
    (Get-Content -LiteralPath $path -Raw).Replace('online_lyrics=true','online_lyrics=false') | Set-Content -LiteralPath $path
}
$results=foreach($state in 'idle','media'){
    $start=[Diagnostics.ProcessStartInfo]::new($executable)
    $start.UseShellExecute=$false
    $start.CreateNoWindow=$true
    $start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.WorkingDirectory=$workspace
    $start.Arguments="--benchmark --demo-state=$state"
    $start.EnvironmentVariables['LOCALAPPDATA']=$profile
    $application=[Diagnostics.Process]::Start($start)
    try{
        Start-Sleep -Seconds $WarmupSeconds
        if($application.HasExited){throw "Application exited before $state measurement"}
        $application.Refresh()
        $cpu=$application.TotalProcessorTime.TotalSeconds
        $clock=[Diagnostics.Stopwatch]::StartNew()
        $samples=@()
        while($clock.Elapsed.TotalSeconds -lt $SampleSeconds){
            if($application.HasExited){throw "Application exited during $state measurement"}
            $application.Refresh()
            $samples += [pscustomobject]@{Private=$application.PrivateMemorySize64;WorkingSet=$application.WorkingSet64}
            Start-Sleep -Milliseconds 100
        }
        $application.Refresh()
        [pscustomobject]@{
            State=$state;Material='solid'
            PrivateMiB=[math]::Round(($samples.Private|Measure-Object -Maximum).Maximum/1MB,2)
            WorkingSetMiB=[math]::Round(($samples.WorkingSet|Measure-Object -Maximum).Maximum/1MB,2)
            CpuOneCorePercent=[math]::Round(100*($application.TotalProcessorTime.TotalSeconds-$cpu)/$clock.Elapsed.TotalSeconds,3)
            SampleSeconds=[math]::Round($clock.Elapsed.TotalSeconds,3)
        }
    }finally{
        if(-not $application.HasExited){
            $null=$application.CloseMainWindow()
            if(-not $application.WaitForExit(2000)){Stop-Process -Id $application.Id}
        }
    }
}
$directory=Join-Path $workspace 'verification'
New-Item -ItemType Directory -Force -Path $directory | Out-Null
$file=if($DisableOnlineLyrics){'lyrics-no-network-performance.csv'}else{'lyrics-steady-performance.csv'}
$results | Export-Csv -LiteralPath (Join-Path $directory $file) -NoTypeInformation
$results | Format-Table
