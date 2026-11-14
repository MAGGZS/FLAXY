# Baixa o yt-dlp.exe (release oficial) para empacotar no instalador do FLAXY.
# Uso: powershell -ExecutionPolicy Bypass -File tools\fetch-ytdlp.ps1
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root "crates\flaxy-desktop\yt-dlp"
New-Item -ItemType Directory -Force $dest | Out-Null
Invoke-WebRequest -Uri "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe" -OutFile "$dest\yt-dlp.exe" -UseBasicParsing
& "$dest\yt-dlp.exe" --version
