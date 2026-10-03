$ErrorActionPreference = 'Stop'
$launchArguments = @($args)
$taskExecutable = Join-Path $PSScriptRoot 'target\release\hastur-retouch.exe'

# Start-Process joins ArgumentList into a Windows command line. Quote each argument
# explicitly so photograph paths with spaces, quotes, or trailing slashes stay intact.
function ConvertTo-HasturNativeArgument([string]$Argument) {
    if ($Argument.Length -gt 0 -and $Argument -notmatch '[\s"]') { return $Argument }
    $escaped = [regex]::Replace($Argument, '(\\*)"', '$1$1\"')
    $escaped = [regex]::Replace($escaped, '(\\+)$', '$1$1')
    return '"' + $escaped + '"'
}

if (-not (Test-Path -LiteralPath $taskExecutable -PathType Leaf)) {
    Push-Location -LiteralPath $PSScriptRoot
    try {
        cargo build --release --locked --bin hastur-retouch
        if ($LASTEXITCODE -ne 0) { throw 'Hastur Retouch could not be built.' }
    } finally { Pop-Location }
}

$launchOptions = @{
    FilePath = $taskExecutable
    WorkingDirectory = $PSScriptRoot
    WindowStyle = 'Normal'
}
if ($launchArguments.Count -gt 0) {
    $launchOptions.ArgumentList = @($launchArguments | ForEach-Object { ConvertTo-HasturNativeArgument $_ })
}
Start-Process @launchOptions
