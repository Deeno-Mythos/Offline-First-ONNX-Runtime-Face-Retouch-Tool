param([switch]$Backends, [switch]$SkipDownload, [switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$validationRoot = Split-Path -Parent $PSScriptRoot
Push-Location $validationRoot
try {
    $executable = Join-Path $validationRoot 'target/release/examples/runtime_validation.exe'
    if ($SkipBuild -and !(Test-Path -LiteralPath $executable -PathType Leaf)) {
        throw "Missing prebuilt validation executable: $executable. Build it before using -SkipBuild."
    }
    $sourceDir = Join-Path $validationRoot 'output/portrait-validation/sources'
    New-Item -ItemType Directory -Force -Path $sourceDir | Out-Null
    $fixtures = @(
        @{ Name = 'astronaut-public-domain.png'; Uri = 'https://raw.githubusercontent.com/scikit-image/scikit-image/v0.25.2/skimage/data/astronaut.png'; Hash = '88431CD9653CCD539741B555FB0A46B61558B301D4110412B5BC28B5E3EA6CB5' },
        @{ Name = 'grace-hopper-public-domain.jpg'; Uri = 'https://raw.githubusercontent.com/matplotlib/matplotlib/main/lib/matplotlib/mpl-data/sample_data/grace_hopper.jpg'; Hash = 'A8CA6D734765703B09728AB47FE59F473D93AE3967FC24C7C0288C3C7ADB7130' },
        @{ Name = 'william-stitt-cc0.jpg'; Uri = 'https://upload.wikimedia.org/wikipedia/commons/0/04/Face_portrait_%28Unsplash%29.jpg'; Hash = '7356DAA8FD4AD53B946CE0036F06B014431DC89B7AE29ECD8EF18FC54EDCE6B5' }
    )
    foreach ($fixture in $fixtures) {
        $sourcePath = Join-Path $sourceDir $fixture.Name
        if (!(Test-Path -LiteralPath $sourcePath)) {
            if ($SkipDownload) { throw "Missing validation fixture: $sourcePath" }
            Invoke-WebRequest -Uri $fixture.Uri -OutFile $sourcePath
        }
        if ((Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash -ne $fixture.Hash) {
            throw "Fixture hash mismatch: $sourcePath. Preserve the file and inspect its provenance."
        }
    }
    if (!$SkipBuild) {
        cargo build --release --locked --example runtime_validation
        if ($LASTEXITCODE -ne 0) { throw 'Release validation build failed' }
    }
    $cases = @(
        @{ Name = 'demo'; Input = 'assets/demo-portrait.png' },
        @{ Name = 'astronaut'; Input = Join-Path $sourceDir 'astronaut-public-domain.png' },
        @{ Name = 'grace-hopper'; Input = Join-Path $sourceDir 'grace-hopper-public-domain.jpg' },
        @{ Name = 'william-stitt'; Input = Join-Path $sourceDir 'william-stitt-cc0.jpg' }
    )
    $suppliedPath = Join-Path $validationRoot 'example-img/CTU DUMANJUG ORG 09.28.26_JAMESBRO-522.JPG'
    if (Test-Path -LiteralPath $suppliedPath) { $cases += @{ Name = 'supplied'; Input = $suppliedPath } }
    foreach ($case in $cases) {
        & $executable cpu $case.Input "output/portrait-validation/$($case.Name)" 1 native
        if ($LASTEXITCODE -ne 0) { throw "Native validation failed: $($case.Name)" }
    }
    if ($Backends) {
        foreach ($backend in @('cpu', 'burn')) {
            & $executable $backend assets/demo-portrait.png "output/portrait-validation/backend-$backend" 2
            if ($LASTEXITCODE -ne 0) { throw "Backend validation failed: $backend" }
        }
        $pythonPath = Join-Path $validationRoot '.ai-tools/Scripts/python.exe'
        & $pythonPath scripts/compare-ai-backends.py output/portrait-validation/backend-cpu output/portrait-validation/backend-burn output/portrait-validation/backend-comparison.json
        if ($LASTEXITCODE -ne 0) { throw 'Backend output comparison failed' }
    }
} finally { Pop-Location }
