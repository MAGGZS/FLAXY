@echo off
rem FLAXY v2: servidor Rust + frontend web/ servido na mesma porta.
cd /d "%~dp0"
if not exist web\dist\index.html (
  pushd web
  call npm install --no-fund --no-audit
  call npm run build
  popd
)
set FLAXY_STATIC_DIR=%~dp0web\dist
set FLAXY_HOST=127.0.0.1
echo FLAXY em http://127.0.0.1:8000
cargo run --release -p flaxy-server
