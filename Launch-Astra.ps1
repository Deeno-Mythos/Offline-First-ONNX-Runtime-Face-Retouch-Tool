$ErrorActionPreference = 'Stop'
# Compatibility entry point for existing shortcuts and scripts.
& (Join-Path $PSScriptRoot 'Launch-Hastur.ps1') @args
