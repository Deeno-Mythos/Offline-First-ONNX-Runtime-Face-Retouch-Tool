$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot
$taskExecutable = Join-Path $PSScriptRoot 'target\release\astra-retouch.exe'
if (-not (Test-Path -LiteralPath $taskExecutable)) {
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Astra Retouch could not be built.' }
}
Start-Process -FilePath $taskExecutable -WorkingDirectory $PSScriptRoot -WindowStyle Normal
