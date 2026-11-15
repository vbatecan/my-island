param([string]$Work,[string]$Executable,[int]$ParentPid)
$ErrorActionPreference='Stop'
try {
    $payload=if(Test-Path -LiteralPath "$Work/setup.exe"){"$Work/setup.exe"}else{"$Work/extracted/still-island.exe"}
    $ready=Get-Content -LiteralPath "$Work/ready.json" -Raw | ConvertFrom-Json
    if((Get-Item -LiteralPath $payload).Length -ne $ready.bytes -or (Get-FileHash -LiteralPath $payload -Algorithm SHA256).Hash -ine $ready.sha256){throw 'Update verification failed'}
    Wait-Process -Id $ParentPid -Timeout 60 -ErrorAction SilentlyContinue
    if(Get-Process -Id $ParentPid -ErrorAction SilentlyContinue){throw 'Application is still running'}
    if(Test-Path -LiteralPath "$Work/setup.exe"){
        $process=Start-Process -FilePath "$Work/setup.exe" -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',('/DIR="'+(Split-Path $Executable)+'"')) -Wait -PassThru -WindowStyle Hidden
        if($process.ExitCode -ne 0){throw 'Installer failed'}
    }else{
        $backup=$Executable+'.update-backup'
        Move-Item -LiteralPath $Executable -Destination $backup -Force
        try{Copy-Item -LiteralPath "$Work/extracted/still-island.exe" -Destination $Executable}
        catch{Move-Item -LiteralPath $backup -Destination $Executable -Force;throw}
    }
    Start-Process -FilePath $Executable -WorkingDirectory (Split-Path $Executable)
}catch{
    $_.Exception.Message | Set-Content -LiteralPath "$Work/install-error.txt"
    Start-Process -FilePath $Executable -WorkingDirectory (Split-Path $Executable)
}
