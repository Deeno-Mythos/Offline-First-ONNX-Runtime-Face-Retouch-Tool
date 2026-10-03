$ErrorActionPreference = 'Stop'
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    python scripts/setup-ai.py --runtime-only
    if ($LASTEXITCODE -ne 0) { throw 'DirectML runtime setup failed.' }
} finally { Pop-Location }
