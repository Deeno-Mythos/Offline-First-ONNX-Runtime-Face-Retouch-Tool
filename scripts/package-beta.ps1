[CmdletBinding()]
param(
    [string]$Iscc,
    [string]$Python = 'python',
    [string]$VcRedistDirectory,
    [string]$SignTool,
    [string]$CertificateThumbprint,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$taskRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$config = Get-Content -LiteralPath (Join-Path $taskRoot 'release/beta.json') -Raw | ConvertFrom-Json
$cargoText = Get-Content -LiteralPath (Join-Path $taskRoot 'Cargo.toml') -Raw
$version = [regex]::Match($cargoText, '(?m)^version = "([^"]+)"').Groups[1].Value
if ($version -notmatch '^\d+\.\d+\.\d+-beta\.\d+$') { throw 'Expected a beta package version.' }
if (-not $Iscc) {
    $candidate = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($candidate) { $Iscc = $candidate.Source }
    else { $Iscc = Join-Path $taskRoot 'output/beta-prep/tools/inno/ISCC.exe' }
}
if (-not (Test-Path -LiteralPath $Iscc -PathType Leaf)) { throw 'Install Inno Setup 6.7.3 and pass -Iscc with its ISCC.exe path.' }
if (-not $VcRedistDirectory) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) { throw 'Pass -VcRedistDirectory with the x64 Microsoft.VC143.CRT redistributable directory.' }
    $vsRoot = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    $redistRoot = Join-Path $vsRoot 'VC/Redist/MSVC'
    $versionDirectory = Get-ChildItem -LiteralPath $redistRoot -Directory | Where-Object { $_.Name -match '^\d+\.\d+\.\d+$' } | Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    $VcRedistDirectory = Join-Path $versionDirectory.FullName 'x64/Microsoft.VC143.CRT'
}
$VcRedistDirectory = (Resolve-Path -LiteralPath $VcRedistDirectory).Path
# Create a new directory for every build. Never delete an existing package or outputs.
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$buildRoot = Join-Path $taskRoot "output/beta/$version/$stamp"
$payload = Join-Path $buildRoot 'payload'
$artifacts = Join-Path $buildRoot 'artifacts'
New-Item -ItemType Directory -Path $payload,$artifacts -Force | Out-Null
Push-Location -LiteralPath $taskRoot
try {
    if (-not $SkipBuild) {
        & cargo build --release --locked --bin hastur-retouch -j2
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    }
    $executable = Join-Path $taskRoot 'target/release/hastur-retouch.exe'
    if ((Get-Item -LiteralPath $executable).VersionInfo.ProductVersion -ne $version) { throw 'Executable version does not match Cargo.toml; rebuild before packaging.' }
    Copy-Item -LiteralPath $executable -Destination (Join-Path $payload 'hastur-retouch.exe')
    $fileList = Get-Content -LiteralPath 'release/package-files.json' -Raw | ConvertFrom-Json
    $burnProvenance=@{}
    foreach($line in (Get-Content -LiteralPath 'models/burn/provenance.txt')) {
        if($line -match '^([a-f0-9]{64}) ([a-f0-9]{64}) ([a-zA-Z0-9_]+)$') {
            $burnProvenance[$Matches[3]]=@{sourceHash=$Matches[1];packHash=$Matches[2]}
        }
    }
    foreach ($entry in $fileList.files) {
        if ($entry.source -match '(^|[\\/])\.\.([\\/]|$)' -or [IO.Path]::IsPathRooted($entry.source) -or [IO.Path]::IsPathRooted($entry.destination) -or $entry.destination -match '(^|[\\/])\.\.([\\/]|$)') { throw 'Unsafe package path.' }
        $source = Join-Path $taskRoot $entry.source
        $sourceHash=(Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
        if($entry.PSObject.Properties.Name -contains 'generatedFrom') {
            $modelName=[IO.Path]::GetFileNameWithoutExtension($entry.source)
            if(-not $burnProvenance.ContainsKey($modelName)){throw "Missing generated Burn provenance: $modelName"}
            $record=$burnProvenance[$modelName]
            if($sourceHash -ne $record.packHash -or (Get-FileHash -LiteralPath $entry.generatedFrom -Algorithm SHA256).Hash.ToLowerInvariant() -ne $record.sourceHash){throw "Burn pack does not match its build provenance: $modelName"}
        } elseif ($sourceHash -ne $entry.sha256) { throw "Bundled asset changed: $($entry.source). Review and update the allowlist hashes." }
        $destination = Join-Path $payload $entry.destination
        New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
        Copy-Item -LiteralPath $source -Destination $destination
    }
    Copy-Item -LiteralPath 'models/burn/provenance.txt' -Destination (Join-Path $payload 'models/burn/provenance.txt')
    $crtNames = @('concrt140.dll','msvcp140.dll','msvcp140_1.dll','msvcp140_2.dll','msvcp140_atomic_wait.dll','msvcp140_codecvt_ids.dll','vccorlib140.dll','vcruntime140.dll','vcruntime140_1.dll','vcruntime140_threads.dll')
    foreach ($name in $crtNames) {
        $source = Join-Path $VcRedistDirectory $name
        $signature = Get-AuthenticodeSignature -LiteralPath $source
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Microsoft') { throw "Unverified Microsoft redistributable: $source" }
        Copy-Item -LiteralPath $source -Destination (Join-Path $payload $name)
    }
    $metadata = Join-Path $buildRoot 'cargo-metadata.json'
    $tree = Join-Path $buildRoot 'cargo-tree.txt'
    & cargo metadata --locked --offline --format-version 1 --filter-platform x86_64-pc-windows-msvc | Set-Content -LiteralPath $metadata -Encoding UTF8
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    & cargo tree --locked --offline --target x86_64-pc-windows-msvc --edges normal,build --prefix none --format '{p}' | Set-Content -LiteralPath $tree -Encoding UTF8
    if ($LASTEXITCODE -ne 0) { throw 'Cargo tree failed.' }
    $licenses = Join-Path $payload 'licenses'
    & $Python scripts/release-notices.py $metadata $tree $licenses
    if ($LASTEXITCODE -ne 0) { throw 'Dependency notice collection failed.' }
    $vsLicenseRoot = (Get-Item -LiteralPath $VcRedistDirectory).Parent.Parent.Parent.Parent.Parent.Parent.FullName
    foreach ($name in @('Redist.txt','ThirdPartyNotices.txt')) {
        $source = Join-Path $vsLicenseRoot "Licenses/1033/$name"
        $label = if ($name -eq 'Redist.txt') { 'Visual-Studio-Redist.txt' } else { 'Visual-Studio-ThirdPartyNotices.txt' }
        Copy-Item -LiteralPath $source -Destination (Join-Path $licenses $label)
    }
    Copy-Item -LiteralPath (Join-Path (Split-Path -Parent $Iscc) 'License.txt') -Destination (Join-Path $licenses 'Inno-Setup-license.txt')
    foreach ($name in @('Quick-start.html','Third-party-notices.txt')) {
        $text = (Get-Content -LiteralPath "release/$name" -Raw).Replace('0.1.0-beta.1', $version)
        Set-Content -LiteralPath (Join-Path $payload $name) -Value $text -Encoding UTF8
    }
    if ($CertificateThumbprint) {
        if (-not $SignTool -or -not (Test-Path -LiteralPath $SignTool)) { throw 'Pass -SignTool when signing a release.' }
        if ($CertificateThumbprint -notmatch '^[0-9A-Fa-f]{40,64}$') { throw 'Invalid certificate thumbprint.' }
        & $SignTool sign /sha1 $CertificateThumbprint /fd SHA256 /tr 'http://timestamp.digicert.com' /td SHA256 (Join-Path $payload 'hastur-retouch.exe')
        if ($LASTEXITCODE -ne 0) { throw 'Application signing failed.' }
    }
    & $Python scripts/verify-package.py $payload
    if ($LASTEXITCODE -ne 0) { throw 'Package dependency/architecture verification failed.' }
    $installedFiles = @(Get-ChildItem -LiteralPath $payload -Recurse -File | Sort-Object FullName | ForEach-Object {
        @{path=$_.FullName.Substring($payload.Length + 1).Replace('\','/'); bytes=$_.Length; sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
    })
    @{schema=1;version=$version;platform='windows-x64';files=$installedFiles} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $payload 'installed-files.json') -Encoding UTF8
    $numericVersion = $version.Replace('-beta.', '.')
    $compilerArgs = @("/DPayloadRoot=$payload", "/DArtifactsRoot=$artifacts", "/DAppVersion=$version", "/DNumericVersion=$numericVersion", "/DPublisher=$($config.publisher)")
    if ($CertificateThumbprint) {
        $signCommand = '$q' + $SignTool + '$q sign /sha1 ' + $CertificateThumbprint + ' /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $f'
        $compilerArgs += @('/DSignRelease=1', "/Shastur_release=$signCommand")
    }
    & $Iscc @compilerArgs (Join-Path $taskRoot 'release/hastur.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed.' }
    $setup = Get-ChildItem -LiteralPath $artifacts -Filter '*-setup.exe' | Select-Object -First 1
    $signature = Get-AuthenticodeSignature -LiteralPath $setup.FullName
    if ($CertificateThumbprint -and $signature.Status -ne 'Valid') { throw 'Compiled installer signature is not valid.' }
    $gitCommit = (& git rev-parse HEAD).Trim()
    $gitDirty = [bool](& git status --porcelain)
    $manifest = @{schema=1;version=$version;releaseDate=$config.releaseDate;platform='windows-x64';publisher=$config.publisher;feedbackEmail=$config.feedbackEmail;gitCommit=$gitCommit;gitDirty=$gitDirty;signingStatus=$signature.Status.ToString();distributionRightsConfirmed=$config.distributionRightsConfirmed;installer=@{file=$setup.Name;bytes=$setup.Length;sha256=(Get-FileHash -LiteralPath $setup.FullName -Algorithm SHA256).Hash.ToLowerInvariant()};installedFilesManifestSha256=(Get-FileHash -LiteralPath (Join-Path $payload 'installed-files.json') -Algorithm SHA256).Hash.ToLowerInvariant()}
    $manifest | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $artifacts 'release-manifest.json') -Encoding UTF8
    "$($manifest.installer.sha256)  $($setup.Name)" | Set-Content -LiteralPath (Join-Path $artifacts 'SHA256SUMS.txt') -Encoding ASCII
    Copy-Item -LiteralPath 'release/Quick-start.html','release/beta-info.txt' -Destination $artifacts
    Copy-Item -LiteralPath 'scripts/verify-installed-beta.ps1' -Destination $artifacts
    Write-Output "Beta installer: $($setup.FullName)"
    Write-Output "Build directory: $buildRoot"
} finally { Pop-Location }
