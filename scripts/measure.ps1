param([string]$Executable = "$PSScriptRoot\..\target\release\still-island.exe", [string]$Appearance="solid")
$ErrorActionPreference = 'Stop'
Set-Content -LiteralPath "$PSScriptRoot\..\verification.log" -Value ''
$process = Start-Process -FilePath (Resolve-Path $Executable) -ArgumentList '--verify','--demo',"--appearance=$Appearance" -WindowStyle Hidden -PassThru -WorkingDirectory (Resolve-Path "$PSScriptRoot\..")
$clock = [Diagnostics.Stopwatch]::StartNew()
$rows = [Collections.Generic.List[object]]::new()
while (-not $process.HasExited) {
    $process.Refresh()
    $seconds = $clock.Elapsed.TotalSeconds
    $phase = if ($seconds -lt 3) { 'idle' } elseif ($seconds -lt 6) { 'expanded' } elseif ($seconds -lt 9) { 'media' } elseif ($seconds -lt 12) { 'volume' } elseif ($seconds -lt 14) { 'interruptions' } else { 'settled' }
    $rows.Add([pscustomobject]@{ Seconds=[math]::Round($seconds,3); Phase=$phase; PrivateBytes=$process.PrivateMemorySize64; WorkingSet=$process.WorkingSet64; CpuSeconds=$process.TotalProcessorTime.TotalSeconds })
    Start-Sleep -Milliseconds 100
}
$rows | Export-Csv "$PSScriptRoot\..\performance.csv" -NoTypeInformation
foreach ($phase in 'idle','expanded','media','volume','interruptions','settled') {
    # Exclude the first second of each steady phase; interruptions use every sample.
    $samples = @($rows | Where-Object Phase -eq $phase)
    if ($phase -ne 'interruptions' -and $samples.Count -gt 15) { $samples = @($samples | Select-Object -Skip 10) }
    if ($samples.Count -lt 2) { continue }
    $first=$samples[0];$last=$samples[-1]
    [pscustomobject]@{ Phase=$phase; PrivateMiB=[math]::Round(($samples.PrivateBytes | Measure-Object -Maximum).Maximum / 1MB,2); WorkingSetMiB=[math]::Round(($samples.WorkingSet | Measure-Object -Maximum).Maximum / 1MB,2); CpuOneCorePercent=[math]::Round(100*($last.CpuSeconds-$first.CpuSeconds)/($last.Seconds-$first.Seconds),2) }
}
Write-Output "Exit code: $($process.ExitCode). CPU percentage uses one logical core as 100%. GPU not sampled."

$verification = Get-Content "$PSScriptRoot\..\verification.log" -Raw
if ($verification -match 'FAILED' -or $verification -notmatch 'complete frames=\d+ active=false') { throw 'Runtime verification failed; inspect verification.log' }
