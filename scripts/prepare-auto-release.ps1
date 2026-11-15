$ErrorActionPreference='Stop'
git fetch origin --tags
if($LASTEXITCODE){throw 'Could not fetch tags'}
$source=$env:GITHUB_SHA
$existing=git tag --list 'v*' --contains $source
foreach($tag in $existing){
    if($tag -match '^v\d+\.\d+\.\d+$' -and ((git rev-parse "$tag^{commit}") -eq $source -or (git rev-parse "$tag^1") -eq $source)){
        "tag=$tag" >> $env:GITHUB_OUTPUT
        exit 0
    }
}
$current=(& "$PSScriptRoot/release-version.ps1").version
$versions=@(git tag --list 'v*' | Where-Object {$_ -match '^v\d+\.\d+\.\d+$'} | ForEach-Object {[version]$_.Substring(1)} | Sort-Object)
$latest=if($versions.Count){$versions[-1]}else{[version]'0.0.0'}
$version=if([version]$current -gt $latest){$current}else{'{0}.{1}.{2}' -f $latest.Major,$latest.Minor,($latest.Build+1)}
$text=[IO.File]::ReadAllText('Cargo.toml')
$text=[regex]::Replace($text,'(?m)^version = "'+[regex]::Escape($current)+'"',('version = "'+$version+'"'),1)
[IO.File]::WriteAllText((Join-Path $PWD 'Cargo.toml'),$text)
cargo update --workspace
if($LASTEXITCODE){throw 'Lockfile update failed'}
& "$PSScriptRoot/release-version.ps1" -Tag "v$version" | Out-Null
git config user.name 'github-actions[bot]'
git config user.email '41898282+github-actions[bot]@users.noreply.github.com'
git add Cargo.toml Cargo.lock
git commit --allow-empty -m "Release v$version from $source"
if($LASTEXITCODE){throw 'Release commit failed'}
git tag "v$version"
git push origin "refs/tags/v$version"
if($LASTEXITCODE){throw 'Release tag push failed'}
"tag=v$version" >> $env:GITHUB_OUTPUT
