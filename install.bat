@echo off
setlocal
cd /d %~dp0

echo ============================================
echo  FLAXY - Script de Instalacao
echo ============================================

echo.
echo [1/5] Verificando Python...
:: Sem Python instalado, o Windows responde "python" com um atalho da Microsoft
:: Store que so imprime uma mensagem e falha. O errorlevel cobre os dois casos.
python --version >nul 2>&1
if errorlevel 1 (
    echo ERRO: Python nao encontrado. Instale Python 3.11+ em https://python.org
    echo Na instalacao, marque "Add python.exe to PATH".
    pause & exit /b 1
)
python --version

echo.
echo [2/5] Verificando Node.js...
call npm --version >nul 2>&1
if errorlevel 1 (
    echo ERRO: Node.js nao encontrado. Instale Node 18+ em https://nodejs.org
    pause & exit /b 1
)
node --version

echo.
echo [3/5] Verificando FFmpeg...
ffmpeg -version >nul 2>&1
if errorlevel 1 (
    echo AVISO: FFmpeg nao encontrado no PATH.
    echo Baixe em https://www.gyan.dev/ffmpeg/builds/ e adicione a pasta bin ao PATH.
    echo O FLAXY sobe sem ele, mas nenhum video vai ser processado.
    echo Pressione qualquer tecla para continuar mesmo assim...
    pause >nul
)

echo.
echo [4/5] Instalando dependencias Python em backend\.venv...
:: Ambiente virtual isola as versoes do requirements.txt das de outros projetos
:: da maquina. start.bat usa este Python automaticamente.
if not exist backend\.venv\Scripts\python.exe (
    python -m venv backend\.venv
    if errorlevel 1 (
        echo ERRO: nao foi possivel criar o ambiente virtual.
        pause & exit /b 1
    )
)
backend\.venv\Scripts\python.exe -m pip install --upgrade pip
backend\.venv\Scripts\python.exe -m pip install -r backend\requirements.txt
if errorlevel 1 (
    echo ERRO: falha ao instalar as dependencias Python.
    pause & exit /b 1
)

:: Configuracao local. Nunca sobrescreve um .env que ja existe.
if not exist backend\.env copy backend\.env.example backend\.env >nul

echo.
echo [5/5] Instalando dependencias Node.js...
pushd frontend
call npm install
if errorlevel 1 (
    popd
    echo ERRO: falha no npm install.
    pause & exit /b 1
)
popd

echo.
echo ============================================
echo  Instalacao concluida!
echo  Execute start.bat para iniciar o FLAXY.
echo ============================================
pause
endlocal
