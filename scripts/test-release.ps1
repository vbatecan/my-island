$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$version=& "$PSScriptRoot/release-version.ps1"
if($version.msix_version -ne "$($version.version).0"){throw 'Version mapping failed'}
& "$PSScriptRoot/release-version.ps1" -Tag "v$($version.version)" | Out-Null
foreach($tag in @('v999.0.0','v01.2.3','v1.2.3-rc1','1.2.3','v1.2.3/../../x')){
    $rejected=$false;try{& "$PSScriptRoot/release-version.ps1" -Tag $tag|Out-Null}catch{$rejected=$true}
    if(-not $rejected){throw "Unsafe or mismatched version accepted: $tag"}
}
$temp=Join-Path ([IO.Path]::GetTempPath()) ('still-release-test-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temp|Out-Null
$output=Join-Path $temp 'outputs'
& "$PSScriptRoot/release-version.ps1" -Tag "v$($version.version)" -OutputFile $output | Out-Null
if((Get-Content -LiteralPath $output -Raw) -notmatch "msix_version=$([regex]::Escape($version.msix_version))"){throw 'GitHub outputs failed'}
$oldPfx=$env:WINDOWS_CERTIFICATE_PFX;$oldPassword=$env:WINDOWS_CERTIFICATE_PASSWORD;$oldPublisher=$env:WINDOWS_SIGNING_PUBLISHER
try{
    $env:WINDOWS_CERTIFICATE_PFX='';$env:WINDOWS_CERTIFICATE_PASSWORD='';$env:WINDOWS_SIGNING_PUBLISHER=''
    $unsigned=& "$PSScriptRoot/sign-release.ps1" -Paths @()
    if($unsigned.signing_enabled){throw 'Signing falsely enabled'}
    $env:WINDOWS_CERTIFICATE_PASSWORD='test-placeholder'
    $rejected=$false;try{& "$PSScriptRoot/sign-release.ps1" -Paths @()|Out-Null}catch{$rejected=$true}
    if(-not $rejected){throw 'Partial signing config accepted'}
}finally{$env:WINDOWS_CERTIFICATE_PFX=$oldPfx;$env:WINDOWS_CERTIFICATE_PASSWORD=$oldPassword;$env:WINDOWS_SIGNING_PUBLISHER=$oldPublisher}
Write-Output 'PASS release tag validation, version mapping, output encoding, unsigned mode, and partial-secret rejection'
