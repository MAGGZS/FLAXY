; Instalador do FLAXY (NSIS, Modern UI 2).
; Gera target\release\FLAXY-Setup-<versão>.exe a partir de:
;   target\release\FLAXY.exe   app nativo
;   bin\                        ffmpeg, ffprobe, yt-dlp, whisper\ (o que existir)
; Instala por usuário (sem pedir administrador) em %LOCALAPPDATA%\Programs\FLAXY.
; Use build-installer.bat em vez de chamar este script direto.

Unicode true
!ifndef VERSION
  !define VERSION "2.0.0"
!endif
!define APP "FLAXY"
!define EXE "FLAXY.exe"
!define ROOT ".."
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"

Name "${APP}"
OutFile "${ROOT}\target\release\FLAXY-Setup-${VERSION}.exe"
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "Software\${APP}" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetCompressorDictSize 64
BrandingText "${APP} ${VERSION}"
ShowInstDetails nevershow
ShowUninstDetails nevershow

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "FileDescription" "Instalador do ${APP}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "${APP}"

!include "MUI2.nsh"
!include "FileFunc.nsh"

!define MUI_ICON "${ROOT}\crates\flaxy-app\assets\icon.ico"
!define MUI_UNICON "${ROOT}\crates\flaxy-app\assets\icon.ico"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TITLE "Instalar o ${APP} ${VERSION}"
!define MUI_WELCOMEPAGE_TEXT "Editor de vídeos em massa: modelos, lotes, legendas automáticas e clipes de retenção.$\r$\n$\r$\nO FFmpeg e as ferramentas de vídeo já vêm junto. Nada mais precisa ser instalado.$\r$\n$\r$\nClique em Avançar para continuar."
!define MUI_FINISHPAGE_TITLE "${APP} instalado"
!define MUI_FINISHPAGE_TEXT "O ${APP} está pronto. Ele também fica no Menu Iniciar e na Área de Trabalho."
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Abrir o ${APP} agora"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "PortugueseBR"

Section "FLAXY" SecMain
  SectionIn RO
  ; Fecha uma instância aberta para poder substituir o executável.
  nsExec::Exec 'taskkill /IM ${EXE} /F'
  Sleep 300

  SetOutPath "$INSTDIR"
  File "${ROOT}\target\release\${EXE}"
  SetOutPath "$INSTDIR\bin"
  File /r /x "*.pdb" "${ROOT}\bin\*.*"
  SetOutPath "$INSTDIR\licencas"
  File "${ROOT}\crates\flaxy-app\assets\LICENSE-geist.txt"
  File "${ROOT}\crates\flaxy-app\assets\LICENSE-lucide.txt"
  SetOutPath "$INSTDIR"

  WriteUninstaller "$INSTDIR\Desinstalar.exe"
  CreateDirectory "$SMPROGRAMS\${APP}"
  CreateShortcut "$SMPROGRAMS\${APP}\${APP}.lnk" "$INSTDIR\${EXE}"
  CreateShortcut "$SMPROGRAMS\${APP}\Desinstalar ${APP}.lnk" "$INSTDIR\Desinstalar.exe"
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\${EXE}"

  WriteRegStr HKCU "Software\${APP}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Desinstalar.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Desinstalar.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" "$0"
SectionEnd

; Os dados do usuário (modelos, banco, vídeos prontos) ficam em
; %APPDATA%\FLAXY e Vídeos\FLAXY: desinstalar não apaga o trabalho de ninguém.
Section "Uninstall"
  nsExec::Exec 'taskkill /IM ${EXE} /F'
  Sleep 300
  Delete "$DESKTOP\${APP}.lnk"
  RMDir /r "$SMPROGRAMS\${APP}"
  RMDir /r "$INSTDIR\bin"
  RMDir /r "$INSTDIR\licencas"
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\Desinstalar.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\${APP}"
SectionEnd
