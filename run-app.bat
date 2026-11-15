@echo off
rem Abre o FLAXY em modo de desenvolvimento (usa as ferramentas de bin\).
cd /d "%~dp0"
cargo run -p flaxy-app
