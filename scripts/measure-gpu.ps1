$ErrorActionPreference='Stop'
$process=Start-Process -FilePath "$PSScriptRoot\..\target\release\still-island.exe" -ArgumentList '--verify','--demo' -WindowStyle Hidden -PassThru -WorkingDirectory "$PSScriptRoot\.."
$clock=[Diagnostics.Stopwatch]::StartNew()
$rows=@()
try {
    $counters=Get-Counter '\GPU Engine(*)\Utilization Percentage' -SampleInterval 1 -MaxSamples 18 -ErrorAction SilentlyContinue
    foreach($counter in $counters){
       $samples=@($counter.CounterSamples|Where-Object { $_.InstanceName -like "pid_$($process.Id)_*" -and $_.Status -eq 0 })
       $rows += [pscustomobject]@{Timestamp=$counter.Timestamp.ToString('o');Seconds=($counter.Timestamp-$process.StartTime).TotalSeconds;ProcessId=$process.Id;MaxEnginePercent=if($samples.Count){($samples.CookedValue|Measure-Object -Maximum).Maximum}else{$null};PresentCounters=$samples.Count}
    }
} finally { if(-not $process.HasExited){$null=$process.CloseMainWindow();if(-not $process.WaitForExit(2000)){Stop-Process -Id $process.Id}} }
$rows|Export-Csv "$PSScriptRoot\..\gpu-performance.csv" -NoTypeInformation
$rows|Format-Table
