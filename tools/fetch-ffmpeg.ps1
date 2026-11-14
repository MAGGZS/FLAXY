# Baixa o FFmpeg (build "gpl-shared" do BtbN) para empacotar no instalador do FLAXY.
# ffmpeg.exe e ffprobe.exe dividem as mesmas DLLs, o que deixa o pacote bem menor
# que dois executaveis estaticos. Inclui NVENC, AMF, QSV e libass.
# Uso: powershell -ExecutionPolicy Bypass -File tools\fetch-ffmpeg.ps1
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root "crates\flaxy-desktop\ffmpeg"
$url  = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip"
$tmp  = Join-Path ([IO.Path]::GetTempPath()) ("flaxy-ffmpeg-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $tmp | Out-Null
Write-Host "Baixando $url"
Invoke-WebRequest -Uri $url -OutFile "$tmp\ffmpeg.zip" -UseBasicParsing
Expand-Archive "$tmp\ffmpeg.zip" -DestinationPath $tmp
$bin = Get-ChildItem $tmp -Recurse -Directory -Filter bin | Select-Object -First 1
Get-ChildItem $dest -Exclude LEIA.txt -ErrorAction SilentlyContinue | Remove-Item -Force
New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item "$($bin.FullName)\ffmpeg.exe", "$($bin.FullName)\ffprobe.exe" $dest
Copy-Item "$($bin.FullName)\*.dll" $dest
$lic = Get-ChildItem $tmp -Recurse -File -Filter LICENSE.txt | Select-Object -First 1
if ($lic) { Copy-Item $lic.FullName (Join-Path $dest "LICENSE-ffmpeg.txt") }
Remove-Item -Recurse -Force $tmp
& "$dest\ffmpeg.exe" -hide_banner -version | Select-Object -First 1
"{0:N0} MB em $dest" -f ((Get-ChildItem $dest | Measure-Object Length -Sum).Sum / 1MB)
