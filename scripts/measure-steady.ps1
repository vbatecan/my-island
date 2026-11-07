$ErrorActionPreference='Stop'
$results=foreach($state in 'idle','expanded','media','volume') {
    $process=Start-Process -FilePath "$PSScriptRoot\..\target\release\still-island.exe" -ArgumentList "--demo-state=$state",'--benchmark' -WindowStyle Hidden -PassThru -WorkingDirectory "$PSScriptRoot\.."
    try {
        Start-Sleep -Seconds 6
        if($process.HasExited){throw "Application exited before $state measurement"}
        $process.Refresh();$startCpu=$process.TotalProcessorTime.TotalSeconds;$clock=[Diagnostics.Stopwatch]::StartNew()
        $samples=1..50|ForEach-Object { $process.Refresh();[pscustomobject]@{Private=$process.PrivateMemorySize64;WorkingSet=$process.WorkingSet64};Start-Sleep -Milliseconds 100 }
        $process.Refresh();$elapsed=$clock.Elapsed.TotalSeconds
        [pscustomobject]@{State=$state;PrivateMiB=[math]::Round(($samples.Private|Measure-Object -Maximum).Maximum/1MB,2);WorkingSetMiB=[math]::Round(($samples.WorkingSet|Measure-Object -Maximum).Maximum/1MB,2);CpuOneCorePercent=[math]::Round(100*($process.TotalProcessorTime.TotalSeconds-$startCpu)/$elapsed,2)}
    } finally {if(-not $process.HasExited){$null=$process.CloseMainWindow();if(-not $process.WaitForExit(2000)){Stop-Process -Id $process.Id}}}
}
$results|Export-Csv "$PSScriptRoot\..\steady-performance.csv" -NoTypeInformation
$results|Format-Table

