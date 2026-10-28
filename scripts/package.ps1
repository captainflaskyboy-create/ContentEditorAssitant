param([string]$Binary = 'target/release/acea.exe')
$ErrorActionPreference = 'Stop'
$workspacePath = Split-Path -Parent $PSScriptRoot
Push-Location $workspacePath
try {
    if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) { throw "Missing release executable: $Binary" }
    New-Item -ItemType Directory -Force dist | Out-Null
    Copy-Item -LiteralPath $Binary -Destination dist/acea.exe
    # The executable is the entire runtime. Documentation stays in the repository.
    & ./dist/acea.exe doctor
    if ($LASTEXITCODE -ne 0) { throw 'Native capability check failed' }
    Get-Item -LiteralPath dist/acea.exe | Select-Object FullName,Length
} finally { Pop-Location }
