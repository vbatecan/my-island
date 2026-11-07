[CmdletBinding()]
param([string]$Destination=(Join-Path $PSScriptRoot '../target/tools/inno'))
$ErrorActionPreference='Stop'
$Destination=[IO.Path]::GetFullPath($Destination)
$version='6.7.4'
$hash='A549929319F9120716677C685535CA4671E4B937B5FDC87E2585F32D8EB30E8C'
$compiler=Join-Path $Destination 'ISCC.exe'
if((Test-Path -LiteralPath $compiler) -and (Test-Path -LiteralPath (Join-Path $Destination "compiler-source.sha256")) -and ((Get-Content -LiteralPath (Join-Path $Destination "compiler-source.sha256") -Raw).Trim() -eq $hash)){Write-Output $compiler;return}
$downloadDirectory=Join-Path ([IO.Path]::GetDirectoryName($Destination)) 'downloads'
New-Item -ItemType Directory -Force -Path $downloadDirectory | Out-Null
$setup=Join-Path $downloadDirectory "innosetup-$version.exe"
if(-not(Test-Path -LiteralPath $setup)){
    if(Get-Command gh -ErrorAction SilentlyContinue){
        & gh release download is-6_7_4 --repo jrsoftware/issrc --pattern "innosetup-$version.exe" --dir $downloadDirectory
        if($LASTEXITCODE -ne 0){throw 'Cannot download the pinned official Inno Setup compiler'}
    }else{Invoke-WebRequest -Uri "https://github.com/jrsoftware/issrc/releases/download/is-6_7_4/innosetup-$version.exe" -OutFile $setup -TimeoutSec 120}
}
if((Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash -ne $hash){throw 'Official Inno compiler checksum mismatch'}
# Official portable mode creates no uninstaller or Add/Remove Programs entry.
$process=Start-Process -FilePath $setup -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-','/PORTABLE=1',"/DIR=`"$Destination`"") -WindowStyle Hidden -Wait -PassThru
if($process.ExitCode -ne 0 -or -not(Test-Path -LiteralPath $compiler)){throw 'Portable compiler extraction failed'}
Set-Content -LiteralPath (Join-Path $Destination "compiler-source.sha256") -Value $hash
Write-Output $compiler
