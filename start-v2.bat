@echo off
rem FLAXY v2 no navegador: servidor Rust serve a API e o frontend web/ na mesma porta.
cd /d "%~dp0"
if not exist web\node_modules ( pushd web & call npm install --no-fund --no-audit & popd )
if not exist web\dist\index.html ( pushd web & call npm run build & popd )
set FLAXY_STATIC_DIR=%~dp0web\dist
set FLAXY_HOST=127.0.0.1
rem FFmpeg e yt-dlp locais em bin\ (opcional). Sem eles, o servidor procura no PATH.
if exist bin\ffmpeg.exe set FLAXY_FFMPEG_DIR=%~dp0bin
if exist bin\yt-dlp.exe set FLAXY_YTDLP_DIR=%~dp0bin
if exist bin\whisper-cli.exe set FLAXY_WHISPER_DIR=%~dp0bin
echo FLAXY em http://127.0.0.1:8000
start "" cmd /c "ping -n 6 127.0.0.1 >nul & start http://127.0.0.1:8000"
cargo run --release -p flaxy-server
