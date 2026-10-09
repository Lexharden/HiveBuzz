#!/usr/bin/env bash
# Compila HiveBuzz para producción en Linux o macOS, de principio a fin.
#
#   ./scripts/build.sh                 compila y reúne los instaladores en release/<versión>/
#   ./scripts/build.sh --updater       además genera los archivos firmados del auto-update (necesita la clave privada en .env)
#   ./scripts/build.sh --skip-install  no reinstala dependencias (más rápido si ya las tienes)
#   ./scripts/build.sh --dry-run       solo comprueba el entorno y muestra qué haría
#   ./scripts/build.sh --help
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

UPDATER=0 SKIP_INSTALL=0 DRY=0
for arg in "$@"; do
  case "$arg" in
    --updater) UPDATER=1 ;;
    --skip-install) SKIP_INSTALL=1 ;;
    --dry-run) DRY=1 ;;
    -h|--help) sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Opción desconocida: $arg (usa --help)" >&2; exit 2 ;;
  esac
done

step() { printf '\n\033[1;33m▶ %s\033[0m\n' "$*"; }
ok()   { printf '  \033[32m✔\033[0m %s\n' "$*"; }
fail() { printf '  \033[31m✘ %s\033[0m\n' "$*" >&2; exit 1; }
run()  { if [ "$DRY" = 1 ]; then printf '  (simulado) %s\n' "$*"; else "$@"; fi; }

OS="$(uname -s)"
step "1/5 Comprobando herramientas ($OS)"
command -v bun   >/dev/null || fail "Falta Bun. Instálalo: curl -fsSL https://bun.sh/install | bash"
ok "bun $(bun --version)"
command -v cargo >/dev/null || fail "Falta Rust. Instálalo: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
ok "$(rustc --version)"

if [ "$OS" = "Linux" ]; then
  command -v pkg-config >/dev/null || fail "Falta pkg-config."
  missing=()
  for lib in webkit2gtk-4.1 gtk+-3.0 alsa dbus-1 openssl; do
    pkg-config --exists "$lib" 2>/dev/null || missing+=("$lib")
  done
  if [ "${#missing[@]}" -gt 0 ]; then
    echo "  Faltan bibliotecas del sistema: ${missing[*]}" >&2
    echo "  En Ubuntu/Debian: sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libasound2-dev libdbus-1-dev libssl-dev pkg-config" >&2
    exit 1
  fi
  ok "bibliotecas de Linux"
elif [ "$OS" = "Darwin" ]; then
  xcode-select -p >/dev/null 2>&1 || fail "Faltan las herramientas de Xcode: ejecuta xcode-select --install"
  ok "herramientas de Xcode"
fi

step "2/5 Dependencias"
if [ "$SKIP_INSTALL" = 1 ]; then
  ok "omitido (--skip-install)"
else
  run bun install --frozen-lockfile
  (cd sidecar && run bun install --frozen-lockfile)
fi

step "3/5 Entorno (.env)"
[ -f .env ] || echo "  ℹ️  No hay .env: copia .env.example a .env para activar Spotify, Twitch y la firma."
run bun run app:check

step "4/5 Compilando (la primera vez tarda varios minutos)"
if [ "$UPDATER" = 1 ]; then
  run bun run app:build -- --config src-tauri/tauri.release.conf.json
else
  run bun run app:build
fi

step "5/5 Reuniendo instaladores"
if [ "$DRY" = 1 ]; then
  echo "  (simulado) bun scripts/collect-artifacts.mjs"
  printf '\n\033[1;32mSimulación terminada: el entorno está listo.\033[0m\n'
else
  bun scripts/collect-artifacts.mjs
fi
