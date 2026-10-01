$ErrorActionPreference = 'Stop'
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    python scripts/setup-ai.py
    if ($LASTEXITCODE -ne 0) { throw 'AI model setup failed.' }
} finally { Pop-Location }
