@echo off
rem FLAXY desktop: compila e abre o app em modo dev (janela nativa).
cd /d "%~dp0"
if not exist web\node_modules ( pushd web & call npm install --no-fund --no-audit & popd )
pushd web & call npm run build & popd
cargo run -p flaxy-desktop
