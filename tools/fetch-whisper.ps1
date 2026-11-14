# Baixa o whisper.cpp (whisper-cli + DLLs) para empacotar no instalador do FLAXY.
# Prefere o build com Vulkan (usa GPU AMD/NVIDIA/Intel); sem ele, usa o build de CPU.
# O modelo (.bin) NAO entra no instalador: o app baixa na aba IA.
# Uso: powershell -ExecutionPolicy Bypass -File tools\fetch-whisper.ps1
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root "crates\flaxy-desktop\whisper"
$rel  = Invoke-RestMethod "https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest" -Headers @{ "User-Agent" = "flaxy-build" }
$asset = $rel.assets | Where-Object { $_.name -match "vulkan" -and $_.name -match "x64" -and $_.name -like "*.zip" } | Select-Object -First 1
if (-not $asset) { $asset = $rel.assets | Where-Object { $_.name -eq "whisper-bin-x64.zip" } | Select-Object -First 1 }
if (-not $asset) { throw "Nenhum build Windows x64 encontrado em $($rel.tag_name)" }
Write-Host "whisper.cpp $($rel.tag_name): $($asset.name)"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("flaxy-whisper-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $tmp | Out-Null
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile "$tmp\w.zip" -UseBasicParsing
Expand-Archive "$tmp\w.zip" -DestinationPath $tmp
$cli = Get-ChildItem $tmp -Recurse -File -Filter "whisper-cli.exe" | Select-Object -First 1
if (-not $cli) { throw "whisper-cli.exe nao veio no pacote" }
New-Item -ItemType Directory -Force $dest | Out-Null
Get-ChildItem $dest -Exclude LEIA.txt -ErrorAction SilentlyContinue | Remove-Item -Force
Copy-Item $cli.FullName $dest
Copy-Item (Join-Path $cli.DirectoryName "*.dll") $dest -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $tmp
"{0:N0} MB em $dest" -f ((Get-ChildItem $dest | Measure-Object Length -Sum).Sum / 1MB)
