[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$InstallDirectory,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [switch]$IncludeBurn
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$install=(Resolve-Path -LiteralPath $InstallDirectory).Path
$output=[IO.Path]::GetFullPath($OutputDirectory)
if ($output -eq $install -or $output.StartsWith($install + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Diagnostic outputs must be outside the installation.' }
New-Item -ItemType Directory -Force -Path $output | Out-Null
if(@(Get-ChildItem -LiteralPath $output -Force).Count -ne 0){throw 'Use a new empty diagnostic output directory.'}
$manifest=Get-Content -LiteralPath (Join-Path $install 'installed-files.json') -Raw | ConvertFrom-Json
foreach($entry in $manifest.files) {
    if([IO.Path]::IsPathRooted($entry.path) -or $entry.path -match '(^|[\\/])\.\.([\\/]|$)'){throw 'Unsafe manifest path'}
    $file=Join-Path $install $entry.path
    if((Get-Item -LiteralPath $file).Length -ne $entry.bytes -or (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -ne $entry.sha256){throw "Installed file differs: $($entry.path)"}
}
function ConvertTo-NativeArgument([string]$Argument) {
    if($Argument.Length -gt 0 -and $Argument -notmatch '[\s"]'){return $Argument}
    $escaped=[regex]::Replace($Argument,'(\\*)"','$1$1\"')
    $escaped=[regex]::Replace($escaped,'(\\+)$','$1$1')
    return '"' + $escaped + '"'
}
function Invoke-Installed([string[]]$Arguments) {
    $nativeArguments=@($Arguments | ForEach-Object {ConvertTo-NativeArgument $_})
    $process=Start-Process -FilePath (Join-Path $install 'hastur-retouch.exe') -ArgumentList $nativeArguments -WorkingDirectory $output -WindowStyle Hidden -PassThru
    $timer=[Diagnostics.Stopwatch]::StartNew()
    while(-not $process.WaitForExit(1000)) {
        if($timer.Elapsed.TotalMinutes -gt 10){Stop-Process -Id $process.Id; throw 'Installed check exceeded ten minutes.'}
    }
    if($process.ExitCode -ne 0){throw "Installed app check failed with exit code $($process.ExitCode); inspect $output"}
}
$names=@('HASTUR_MODELS_DIR','ORT_DYLIB_PATH','HASTUR_AI_CACHE_DIR','HASTUR_PREFERENCES_PATH','HASTUR_RECOVERY_DIR')
$saved=@{}
foreach($name in $names){$saved[$name]=[Environment]::GetEnvironmentVariable($name,'Process')}
try {
    # Fresh preferences/recovery/cache and unrelated CWD prevent accidental use of
    # the current editor's saved state. All real user data remains untouched.
    $env:HASTUR_MODELS_DIR=$null
    $env:ORT_DYLIB_PATH=Join-Path $install 'models/onnxruntime.dll'
    $env:HASTUR_AI_CACHE_DIR=Join-Path $output 'ai-cache'
    $env:HASTUR_PREFERENCES_PATH=Join-Path $output 'preferences.ron'
    $env:HASTUR_RECOVERY_DIR=Join-Path $output 'recovery'
    Invoke-Installed @('--verify-installation',(Join-Path $output 'cpu'),'--verify-provider','cpu')
    Invoke-Installed @('--verify-installation',(Join-Path $output 'cpu-fallback'),'--verify-provider','auto')
    $fallback=Get-Content -LiteralPath (Join-Path $output 'cpu-fallback/installation-check.txt') -Raw
    if($fallback -notmatch 'Portrait inference:[^\r\n]*CPU'){throw 'Auto did not report the CPU-only runtime fallback.'}
    $env:ORT_DYLIB_PATH=$null
    Invoke-Installed @('--verify-installation',(Join-Path $output 'auto'),'--verify-provider','auto')
    if($IncludeBurn){Invoke-Installed @('--verify-installation',(Join-Path $output 'burn'),'--verify-provider','burn')}
    Invoke-Installed @('--100','--screenshot',(Join-Path $output 'native-workspace.png'))
    Invoke-Installed @('--size','600x560','--screenshot',(Join-Path $output 'compact-workspace.png'))
    $report=@{version=$manifest.version;verifiedFiles=$manifest.files.Count;cpu=$true;autoCpuFallback=$true;autoBundledRuntime=$true;burn=$IncludeBurn.IsPresent;nativeScreenshot=$true;compactScreenshot=$true;computer=$env:COMPUTERNAME;windows=[Environment]::OSVersion.VersionString;cleanMachineVerified=$false}
    $report | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $output 'verification.json') -Encoding UTF8
    Write-Output "Installed beta verified. Report: $(Join-Path $output 'verification.json')"
} finally {
    foreach($name in $names){[Environment]::SetEnvironmentVariable($name,$saved[$name],'Process')}
}
exit 0
