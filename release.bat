@echo off
rem Compila, gera o instalador e publica a versao de Cargo.toml no GitHub Releases.
rem O FLAXY instalado pega a versao nova em Configuracoes > Verificar atualizacoes.
rem Precisa de GITHUB_TOKEN definido (veja tools\publish-release.ps1).
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File tools\publish-release.ps1 %*
