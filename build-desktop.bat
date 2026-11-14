@echo off
rem Gera FLAXY.exe e instaladores (.msi / setup.exe) em target\release\bundle
rem com FFmpeg, whisper-cli e yt-dlp embutidos. Baixa os tres na primeira vez.
cd /d "%~dp0"
if not exist crates\flaxy-desktop\ffmpeg\ffmpeg.exe (
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\fetch-ffmpeg.ps1 || exit /b 1
)
if not exist crates\flaxy-desktop\whisper\whisper-cli.exe (
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\fetch-whisper.ps1 || exit /b 1
)
if not exist crates\flaxy-desktop\yt-dlp\yt-dlp.exe (
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\fetch-ytdlp.ps1 || exit /b 1
)
cd crates\flaxy-desktop
call ..\..\web\node_modules\.bin\tauri.cmd build
