# Baixa para bin\ os programas que o FLAXY empacota no instalador.
# So baixa o que estiver faltando. Rode com -Force para atualizar tudo.
#   bin\ffmpeg.exe, bin\ffprobe.exe (+ DLLs)  FFmpeg gpl-shared do BtbN (NVENC, AMF, QSV, libass)
#   bin\yt-dlp.exe                            release oficial do yt-dlp
#   bin\whisper\whisper-cli.exe (+ DLLs)      whisper.cpp (Vulkan se houver, senao CPU)
# O modelo do Whisper (.bin) NAO entra no instalador: o app baixa na aba IA.
# Uso: powershell -ExecutionPolicy Bypass -File tools\fetch-tools.ps1 [-Force]
param([switch]$Force)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$root = Split-Path -Parent $PSScriptRoot
$bin = Join-Path $root "bin"
New-Item -ItemType Directory -Force $bin | Out-Null

function New-Tmp {
  $t = Join-Path ([IO.Path]::GetTempPath()) ("flaxy-" + [guid]::NewGuid())
  New-Item -ItemType Directory -Force $t | Out-Null
  $t
}

if ($Force -or -not (Test-Path "$bin\ffmpeg.exe") -or -not (Test-Path "$bin\ffprobe.exe")) {
  $url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip"
  Write-Host "FFmpeg: $url"
  $tmp = New-Tmp
  Invoke-WebRequest -Uri $url -OutFile "$tmp\ffmpeg.zip" -UseBasicParsing
  Expand-Archive "$tmp\ffmpeg.zip" -DestinationPath $tmp
  $src = Get-ChildItem $tmp -Recurse -Directory -Filter bin | Select-Object -First 1
  Copy-Item "$($src.FullName)\ffmpeg.exe", "$($src.FullName)\ffprobe.exe", "$($src.FullName)\*.dll" $bin -Force
  $lic = Get-ChildItem $tmp -Recurse -File -Filter LICENSE.txt | Select-Object -First 1
  if ($lic) { Copy-Item $lic.FullName "$bin\LICENSE-ffmpeg.txt" -Force }
  Remove-Item -Recurse -Force $tmp
}

if ($Force -or -not (Test-Path "$bin\yt-dlp.exe")) {
  Write-Host "yt-dlp: release oficial"
  Invoke-WebRequest -Uri "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe" -OutFile "$bin\yt-dlp.exe" -UseBasicParsing
}

$wdir = Join-Path $bin "whisper"
if ($Force -or -not (Test-Path "$wdir\whisper-cli.exe")) {
  $rel = Invoke-RestMethod "https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest" -Headers @{ "User-Agent" = "flaxy-build" }
  $asset = $rel.assets | Where-Object { $_.name -match "vulkan" -and $_.name -match "x64" -and $_.name -like "*.zip" } | Select-Object -First 1
  if (-not $asset) { $asset = $rel.assets | Where-Object { $_.name -eq "whisper-bin-x64.zip" } | Select-Object -First 1 }
  if (-not $asset) { throw "Nenhum build Windows x64 do whisper.cpp em $($rel.tag_name)" }
  Write-Host "whisper.cpp $($rel.tag_name): $($asset.name)"
  $tmp = New-Tmp
  Invoke-WebRequest -Uri $asset.browser_download_url -OutFile "$tmp\w.zip" -UseBasicParsing
  Expand-Archive "$tmp\w.zip" -DestinationPath $tmp
  $cli = Get-ChildItem $tmp -Recurse -File -Filter "whisper-cli.exe" | Select-Object -First 1
  if (-not $cli) { throw "whisper-cli.exe nao veio no pacote" }
  New-Item -ItemType Directory -Force $wdir | Out-Null
  Copy-Item $cli.FullName $wdir -Force
  Copy-Item (Join-Path $cli.DirectoryName "*.dll") $wdir -Force -ErrorAction SilentlyContinue
  Remove-Item -Recurse -Force $tmp
}

& "$bin\ffmpeg.exe" -hide_banner -version | Select-Object -First 1
& "$bin\yt-dlp.exe" --version
"{0:N0} MB em $bin" -f ((Get-ChildItem $bin -Recurse -File | Measure-Object Length -Sum).Sum / 1MB)
