@echo off
rem Gera o instalador do FLAXY: target\release\FLAXY-Setup-<versao>.exe
rem 1. Compila o app nativo (release otimizado).
rem 2. Empacota o FLAXY.exe + bin\ (ffmpeg, ffprobe, yt-dlp, whisper) com NSIS.
rem Ferramentas faltando em bin\: rode antes tools\fetch-tools.ps1
setlocal
cd /d "%~dp0"

if not exist bin\ffmpeg.exe (
  echo bin\ffmpeg.exe ausente. Baixando ferramentas...
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\fetch-tools.ps1 || exit /b 1
)

cargo build --release -p flaxy-app || exit /b 1

set "MAKENSIS="
where makensis >nul 2>&1 && set "MAKENSIS=makensis"
if not defined MAKENSIS if exist "%ProgramFiles(x86)%\NSIS\makensis.exe" set "MAKENSIS=%ProgramFiles(x86)%\NSIS\makensis.exe"
if not defined MAKENSIS if exist "%ProgramFiles%\NSIS\makensis.exe" set "MAKENSIS=%ProgramFiles%\NSIS\makensis.exe"
if not defined MAKENSIS if exist "%LOCALAPPDATA%\tauri\NSIS\makensis.exe" set "MAKENSIS=%LOCALAPPDATA%\tauri\NSIS\makensis.exe"
if not defined MAKENSIS (
  echo NSIS nao encontrado. Instale em https://nsis.sourceforge.io e rode de novo.
  exit /b 1
)

for /f "tokens=2 delims== " %%v in ('findstr /b "version" Cargo.toml') do set "VER=%%~v"
"%MAKENSIS%" /V2 /DVERSION=%VER% installer\flaxy.nsi || exit /b 1
echo.
echo Instalador pronto: target\release\FLAXY-Setup-%VER%.exe
endlocal
