# Publica uma versão nova do FLAXY no GitHub Releases.
# O botão "Atualizar" em Configurações baixa o FLAXY.exe anexado aqui.
#
# Antes de rodar:
#   1. Aumente `version` em Cargo.toml (ex.: 2.0.0 -> 2.0.1).
#   2. Faça commit e push para o GitHub.
#   3. Defina um token com permissão "Contents: write" no repositório:
#        $env:GITHUB_TOKEN = "ghp_..."
#
# Uso: release.bat  (ou: powershell -File tools\publish-release.ps1 -Notes "o que mudou")

param(
    [string]$Notes = "",
    [string]$Repo = "MAGGZS/FLAXY",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$token = $env:GITHUB_TOKEN
if (-not $token) {
    # Sem GITHUB_TOKEN: usa a credencial que o Git já guarda para o github.com
    # (a mesma do `git push`). Nunca é impressa.
    # A entrada vai por arquivo sem BOM: o pipe do PowerShell pode pôr um BOM
    # na frente e o Git recusa ("credential missing protocol field").
    $in = [IO.Path]::GetTempFileName()
    try {
        [IO.File]::WriteAllText($in, "protocol=https`nhost=github.com`n`n", (New-Object Text.UTF8Encoding $false))
        $cred = cmd /c "git credential fill < `"$in`" 2>nul"
    } finally {
        Remove-Item $in -ErrorAction SilentlyContinue
    }
    $line = $cred | Where-Object { $_ -like "password=*" } | Select-Object -First 1
    if ($line) { $token = $line.Substring(9) }
}
if (-not $token -and (Get-Command gh -ErrorAction SilentlyContinue)) {
    $token = (gh auth token 2>$null)
}
if (-not $token) {
    Write-Host "Defina GITHUB_TOKEN (permissao Contents: write) ou faca login no Git para o github.com." -ForegroundColor Red
    exit 1
}

$ver = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$tag = "v$ver"
Write-Host "Versao: $tag"

$headers = @{
    Authorization          = "Bearer $token"
    Accept                 = "application/vnd.github+json"
    "X-GitHub-Api-Version" = "2022-11-28"
    "User-Agent"           = "FLAXY-release"
}
$api = "https://api.github.com/repos/$Repo"

# Não publica por cima: o app só atualiza se a tag for maior que a instalada.
try {
    Invoke-RestMethod -Headers $headers -Uri "$api/releases/tags/$tag" | Out-Null
    Write-Host "A release $tag ja existe. Aumente 'version' em Cargo.toml." -ForegroundColor Red
    exit 1
} catch {
    if ($_.Exception.Response.StatusCode.value__ -ne 404) { throw }
}

if (-not $SkipBuild) {
    cmd /c build-installer.bat
    if ($LASTEXITCODE -ne 0) { Write-Host "Falha no build." -ForegroundColor Red; exit 1 }
}

$exe = Join-Path $root "target\release\FLAXY.exe"
$setup = Join-Path $root "target\release\FLAXY-Setup-$ver.exe"
if (-not (Test-Path $exe)) { Write-Host "Nao achei $exe" -ForegroundColor Red; exit 1 }

$commit = (git rev-parse HEAD).Trim()
if (-not $Notes) { $Notes = (git log -1 --pretty=%s) }
$body = @{
    tag_name         = $tag
    target_commitish = $commit
    name             = "FLAXY $ver"
    body             = $Notes
    draft            = $false
    prerelease       = $false
} | ConvertTo-Json
Write-Host "Criando release $tag..."
$rel = Invoke-RestMethod -Method Post -Headers $headers -Uri "$api/releases" -Body $body -ContentType "application/json"

function Upload($path) {
    $name = Split-Path -Leaf $path
    $url = "https://uploads.github.com/repos/$Repo/releases/$($rel.id)/assets?name=$name"
    Write-Host "Enviando $name ($([math]::Round((Get-Item $path).Length / 1MB, 1)) MB)..."
    Invoke-RestMethod -Method Post -Headers $headers -Uri $url -InFile $path -ContentType "application/octet-stream" | Out-Null
}

Upload $exe
if (Test-Path $setup) { Upload $setup }

Write-Host ""
Write-Host "Publicado: $($rel.html_url)" -ForegroundColor Green
Write-Host "No FLAXY: Configuracoes > Verificar atualizacoes."
