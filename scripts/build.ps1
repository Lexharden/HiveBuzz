# Compila HiveBuzz para producción en Windows, de principio a fin.
#
#   .\scripts\build.ps1                 compila y reúne los instaladores en release\<versión>\
#   .\scripts\build.ps1 -Updater        además genera los archivos firmados del auto-update (necesita la clave privada en .env)
#   .\scripts\build.ps1 -SkipInstall    no reinstala dependencias (más rápido si ya las tienes)
#   .\scripts\build.ps1 -DryRun         solo comprueba el entorno y muestra qué haría
#
# Si PowerShell bloquea el script, usa scripts\build.cmd (lo ejecuta sin cambiar tu configuración).
[CmdletBinding()]
param(
  [switch]$Updater,
  [switch]$SkipInstall,
  [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

function Step($msg) { Write-Host "`n> $msg" -ForegroundColor Yellow }
function Ok($msg)   { Write-Host "  OK  $msg" -ForegroundColor Green }
function Fail($msg) { Write-Host "  ERROR  $msg" -ForegroundColor Red; exit 1 }

# Ejecuta un comando externo y detiene el script si falla (PowerShell 5.1 no lo hace solo).
function Run {
  param([string]$Exe, [string[]]$Arguments)
  if ($DryRun) { Write-Host "  (simulado) $Exe $($Arguments -join ' ')"; return }
  & $Exe @Arguments
  if ($LASTEXITCODE -ne 0) { Fail "Falló: $Exe $($Arguments -join ' ') (código $LASTEXITCODE)" }
}

Step '1/5 Comprobando herramientas (Windows)'
if (-not (Get-Command bun -ErrorAction SilentlyContinue)) { Fail 'Falta Bun. Instálalo: powershell -c "irm bun.sh/install.ps1 | iex"' }
Ok "bun $(bun --version)"
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { Fail 'Falta Rust. Instálalo desde https://rustup.rs' }
Ok (rustc --version)
# Rust en Windows necesita las herramientas de compilación de Visual Studio (C++).
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path $vswhere) {
  $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
  if ($vs) { Ok 'Herramientas de C++ de Visual Studio' } else { Fail 'Faltan las "Herramientas de compilación de C++" de Visual Studio (https://visualstudio.microsoft.com/visual-cpp-build-tools/).' }
} else {
  Write-Host '  (no se pudo comprobar Visual Studio; si la compilación falla, instala las herramientas de C++)' -ForegroundColor DarkYellow
}

Step '2/5 Dependencias'
if ($SkipInstall) {
  Ok 'omitido (-SkipInstall)'
} else {
  Run 'bun' @('install', '--frozen-lockfile')
  Push-Location sidecar
  try { Run 'bun' @('install', '--frozen-lockfile') } finally { Pop-Location }
}

Step '3/5 Entorno (.env)'
if (-not (Test-Path .env)) { Write-Host '  Aviso: no hay .env; copia .env.example a .env para activar Spotify, Twitch y la firma.' -ForegroundColor DarkYellow }
Run 'bun' @('run', 'app:check')

Step '4/5 Compilando (la primera vez tarda varios minutos)'
if ($Updater) {
  Run 'bun' @('run', 'app:build', '--', '--config', 'src-tauri/tauri.release.conf.json')
} else {
  Run 'bun' @('run', 'app:build')
}

Step '5/5 Reuniendo instaladores'
if ($DryRun) {
  Write-Host '  (simulado) bun scripts/collect-artifacts.mjs'
  Write-Host "`nSimulación terminada: el entorno está listo." -ForegroundColor Green
} else {
  Run 'bun' @('scripts/collect-artifacts.mjs')
}
