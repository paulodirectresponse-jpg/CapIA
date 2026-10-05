<#
.SYNOPSIS
  Checklist guiado de aceitação em MÁQUINA LIMPA (Windows 10 22H2 / Windows 11, perfil novo, SEM
  ferramentas de desenvolvedor). Gera o JSON de resultados que `validate.mjs` confere.

.DESCRIPTION
  Roda NA máquina de teste (não precisa de Node, Rust, Git ou Python: só PowerShell 5.1+, que já vem no
  Windows). Coleta automaticamente: versão do Windows, presença de ferramentas de desenvolvedor,
  SHA-256 e assinatura Authenticode do instalador, tempo de cada passo, tamanho do arquivo exportado e
  se o projeto do usuário sobreviveu à desinstalação. Os passos que dependem de olhos humanos (criar
  projeto, importar, editar, exportar) são PERGUNTADOS: você responde com o que realmente aconteceu.

  Regras:
   - Nunca invente: responda "n" (falhou) ou "x" (não executei) quando for o caso. Um resultado
     negativo honesto vale mais que um positivo falso — e o validador recusa o que não for real.
   - Use um perfil de usuário NOVO e uma máquina sem Rust/Node/pnpm/Git/Python/CMake/Visual Studio e
     sem ffmpeg no PATH (o FFmpeg deve vir EMBUTIDO no instalador).
   - O script NÃO envia nada pela rede. O JSON fica em -OutFile; revise antes de compartilhar (não
     contém chaves; contém o nome do usuário do Windows apenas se você o digitar em notas).

.PARAMETER Installer      Caminho do instalador do release candidate (.exe/.msi).
.PARAMETER ReleaseCandidate  Versão esperada (padrão 0.6.0-rc.1).
.PARAMETER Performer      Seu nome (aparece em attestation.performed_by).
.PARAMETER Kind           physical | vm | ci_image.
.PARAMETER InstallCommand Comando de instalação silenciosa opcional (ex.: '"{0}" /S'; {0} = caminho do instalador).
                          Sem ele, você instala manualmente e confirma.
.PARAMETER UninstallCommand Comando de desinstalação silenciosa opcional; sem ele, desinstale manualmente.
.PARAMETER OutFile        Onde gravar o JSON (padrão: .\clean-machine-results.json).
.PARAMETER DryRun         Só mostra o inventário (SO + ferramentas); não pergunta nada nem grava resultados.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\run-clean-machine.ps1 -Installer .\CapIA_0.6.0-rc.1_x64-setup.exe -Performer "Maria" -Kind physical
#>
[CmdletBinding()]
param(
  [string]$Installer = "",
  [string]$ReleaseCandidate = "0.6.0-rc.1",
  [string]$Performer = "",
  [ValidateSet("physical", "vm", "ci_image")][string]$Kind = "physical",
  [string]$InstallCommand = "",
  [string]$UninstallCommand = "",
  [string]$OutFile = ".\clean-machine-results.json",
  [switch]$DryRun
)
$ErrorActionPreference = "Stop"

function Get-OsInfo {
  $os = Get-CimInstance Win32_OperatingSystem
  $display = (Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion" -ErrorAction SilentlyContinue).DisplayVersion
  [ordered]@{
    caption         = $os.Caption.Trim()
    version         = $os.Version
    build           = [int]$os.BuildNumber
    display_version = "$display"
  }
}

function Test-Tool([string]$Name) { [bool](Get-Command $Name -ErrorAction SilentlyContinue) }

function Get-DevTools {
  # `python` do Windows Store (alias de 0 bytes) também conta como "presente" para o inventário.
  [ordered]@{
    cargo          = Test-Tool "cargo"
    rustc          = Test-Tool "rustc"
    node           = Test-Tool "node"
    npm            = Test-Tool "npm"
    pnpm           = Test-Tool "pnpm"
    git            = Test-Tool "git"
    python         = (Test-Tool "python") -or (Test-Tool "py")
    cmake          = Test-Tool "cmake"
    cl             = Test-Tool "cl"
    ffmpeg_on_path = Test-Tool "ffmpeg"
  }
}

function Ask-Step([string]$Id, [string]$Title, [string]$Instructions) {
  Write-Host ""
  Write-Host "=== PASSO: $Title ($Id) ===" -ForegroundColor Cyan
  Write-Host $Instructions
  do { $a = (Read-Host "Resultado? [s] funcionou  [n] FALHOU  [x] não executei").ToLower() } until ($a -in @("s", "n", "x"))
  $notes = ""
  if ($a -ne "s") { $notes = Read-Host "Descreva o que aconteceu (obrigatório para n/x)" }
  [ordered]@{
    id           = $Id
    result       = @{ s = "passed"; n = "failed"; x = "not_run" }[$a]
    completed_at = (Get-Date).ToUniversalTime().ToString("o")
    notes        = $notes
  }
}

$os = Get-OsInfo
$dev = Get-DevTools
Write-Host "Windows: $($os.caption) $($os.version) ($($os.display_version))"
Write-Host "Ferramentas de desenvolvedor encontradas:"
$dev.GetEnumerator() | ForEach-Object { Write-Host ("  {0,-15} {1}" -f $_.Key, $(if ($_.Value) { "PRESENTE  <-- máquina NÃO limpa" } else { "ausente" })) }
if ($dev.Values -contains $true) {
  Write-Warning "Esta máquina NÃO é limpa. O validador vai recusar este resultado. Use outra máquina/perfil."
}
if ($DryRun) { return }

if (-not $Performer) { $Performer = Read-Host "Seu nome (executor humano)" }
if (-not $Installer -or -not (Test-Path $Installer)) { throw "Informe -Installer com o caminho de um instalador existente." }
$Installer = (Resolve-Path $Installer).Path
$sig = Get-AuthenticodeSignature -FilePath $Installer
$installerInfo = [ordered]@{
  file                = Split-Path $Installer -Leaf
  sha256              = (Get-FileHash -Algorithm SHA256 -Path $Installer).Hash.ToLower()
  signed              = ($sig.Status -eq "Valid")
  authenticode_status = "$($sig.Status)"
}
Write-Host "Instalador: $($installerInfo.file)  sha256=$($installerInfo.sha256)  assinatura=$($installerInfo.authenticode_status)"
if (-not $installerInfo.signed) { Write-Warning "Instalador NÃO assinado: o resultado vale só como teste; não atende o critério 'instalador assinado'." }

$userProj = Join-Path $env:USERPROFILE "Documents\capia-clean-machine-test.capia"
$started = (Get-Date).ToUniversalTime()
$steps = @()

# 1. install ------------------------------------------------------------------------------------------
if ($InstallCommand) {
  $cmd = [string]::Format($InstallCommand, $Installer)
  Write-Host "Instalando: $cmd"
  $p = Start-Process -FilePath "cmd.exe" -ArgumentList "/c $cmd" -Wait -PassThru
  $ok = ($p.ExitCode -eq 0)
  $steps += [ordered]@{ id = "install"; result = $(if ($ok) { "passed" } else { "failed" }); completed_at = (Get-Date).ToUniversalTime().ToString("o"); notes = "exit code $($p.ExitCode)" }
} else {
  $steps += Ask-Step "install" "Instalar" "Execute o instalador ($($installerInfo.file)) e conclua a instalação. Anote avisos do SmartScreen/UAC e se o WebView2 foi instalado/detectado."
}

# 2. launch -------------------------------------------------------------------------------------------
$steps += Ask-Step "launch" "Abrir o CapIA pela primeira vez" "Abra o CapIA pelo atalho. Deve abrir a tela de boas-vindas SEM pedir chave de IA, sem erro de FFmpeg/WebView2. Confirme também a versão em 'Sobre' = $ReleaseCandidate."
$appVersion = Read-Host "Versão exibida pelo app (ex.: $ReleaseCandidate)"

# 3-5. create_project / import / edit -----------------------------------------------------------------
$steps += Ask-Step "create_project" "Criar projeto" "Crie um projeto novo em: $userProj (Novo projeto). Deve aparecer a timeline vazia."
$steps += Ask-Step "import" "Importar mídia" "Importe um vídeo curto e um áudio/imagem qualquer (arquivos seus; o FFmpeg embutido faz a sondagem). Miniaturas/duração devem aparecer."
$steps += Ask-Step "edit" "Editar" "Arraste o vídeo para a timeline, divida (Dividir no playhead), apare e adicione um título. Desfazer/Refazer devem funcionar. Dê play no preview."

# 6. export -------------------------------------------------------------------------------------------
$steps += Ask-Step "export" "Exportar MP4" "Exporte um MP4 (Exportar > MP4 H.264). Se não houver encoder aprovado, anote a mensagem exata (é um resultado legítimo)."
$exportSize = 0
$exportPath = Read-Host "Caminho do arquivo exportado (Enter se não exportou)"
if ($exportPath -and (Test-Path $exportPath)) { $exportSize = (Get-Item $exportPath).Length }
Write-Host "Tamanho do arquivo exportado: $exportSize bytes"

# 7. uninstall ----------------------------------------------------------------------------------------
$projectBefore = Test-Path $userProj
if ($UninstallCommand) {
  Write-Host "Desinstalando: $UninstallCommand"
  $p = Start-Process -FilePath "cmd.exe" -ArgumentList "/c $UninstallCommand" -Wait -PassThru
  $steps += [ordered]@{ id = "uninstall"; result = $(if ($p.ExitCode -eq 0) { "passed" } else { "failed" }); completed_at = (Get-Date).ToUniversalTime().ToString("o"); notes = "exit code $($p.ExitCode)" }
} else {
  $steps += Ask-Step "uninstall" "Desinstalar" "Desinstale o CapIA (Configurações > Aplicativos). Conclua a desinstalação."
}
$projectPreserved = $projectBefore -and (Test-Path $userProj)
Write-Host "Projeto do usuário preservado após desinstalar: $projectPreserved"

$finished = (Get-Date).ToUniversalTime()
$result = [ordered]@{
  schema            = "capia.phase6.clean-machine/1"
  release_candidate = $ReleaseCandidate
  attestation       = [ordered]@{
    performed_by      = $Performer
    results_are_real  = $false
    no_simulation     = $false
    statement         = "Executei este checklist pessoalmente em máquina limpa; os resultados são reais."
  }
  machines          = @([ordered]@{
      id                                     = "{0}-{1}" -f $env:COMPUTERNAME.ToLower(), $started.ToString("yyyyMMdd")
      kind                                   = $Kind
      os                                     = $os
      fresh_user_profile                     = $false
      dev_tools                              = $dev
      installer                              = $installerInfo
      app_version                            = $appVersion
      started_at                             = $started.ToString("o")
      finished_at                            = $finished.ToString("o")
      steps                                  = $steps
      export                                 = [ordered]@{ file_size_bytes = $exportSize }
      user_project_preserved_after_uninstall = [bool]$projectPreserved
    })
}

# Atestados: o script NÃO os marca sozinho. Quem executou confirma conscientemente.
Write-Host ""
$fresh = (Read-Host "Confirma que usou um perfil de usuário NOVO (criado só para este teste)? [s/n]").ToLower() -eq "s"
$real = (Read-Host "Confirma que TODOS os resultados acima são reais e não simulados? [s/n]").ToLower() -eq "s"
$result.machines[0].fresh_user_profile = $fresh
$result.attestation.results_are_real = $real
$result.attestation.no_simulation = $real

$result | ConvertTo-Json -Depth 10 | Set-Content -Path $OutFile -Encoding UTF8
Write-Host ""
Write-Host "Resultados gravados em $OutFile" -ForegroundColor Green
Write-Host "Valide em uma máquina com Node:  node tools/phase6-acceptance/clean-machine/validate.mjs --file `"$OutFile`""
