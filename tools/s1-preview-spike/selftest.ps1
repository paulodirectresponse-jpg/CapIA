<#
  Autoteste da lógica NÃO específica de Windows do run.ps1 (correlação de contadores de GPU por fase e leitura
  segura do relatório). Roda em Windows PowerShell 5.1 ou pwsh:  .\selftest.ps1
  Não executa o harness nem toca no sistema.
#>
$ErrorActionPreference = "Stop"
$src = Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) "run.ps1"
$tokens = $null; $errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($src, [ref]$tokens, [ref]$errors)
if ($errors.Count -gt 0) { throw "run.ps1 tem erros de sintaxe: $($errors[0].Message)" }
# Carrega só as funções puras (sem efeitos colaterais).
foreach ($name in @("G", "Add-GpuToPhase")) {
  $fn = $ast.Find({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name }, $true)
  Invoke-Expression $fn.Extent.Text
}
$fail = 0
function Check([string]$what, $cond) { if ($cond) { Write-Host "ok   $what" } else { Write-Host "FAIL $what" -ForegroundColor Red; $script:fail++ } }

# --- G: acesso seguro a campos ausentes
$o = [pscustomobject]@{ a = [pscustomobject]@{ b = 5 } }
Check "G lê campo aninhado" ((G $o @("a", "b")) -eq 5)
Check "G devolve nulo em caminho ausente (sem exceção)" ($null -eq (G $o @("a", "x", "y")))

# --- Add-GpuToPhase: janela [1000,4000]; app = pids 10,11; amostra fora da janela deve ser ignorada
$phase = [pscustomobject]@{ name = "steady_cpu"; started_utc_ms = 1000; ended_utc_ms = 4000 }
$samples = @(
  [pscustomobject]@{ t = 500;  tree = @(10, 11); rows = @([pscustomobject]@{ pid = 10; engtype = "3D"; v = 99.0 }) },   # antes da fase
  [pscustomobject]@{ t = 1500; tree = @(10, 11); rows = @([pscustomobject]@{ pid = 10; engtype = "3D"; v = 10.0 }, [pscustomobject]@{ pid = 11; engtype = "3D"; v = 5.0 }, [pscustomobject]@{ pid = 99; engtype = "3D"; v = 40.0 }) },
  [pscustomobject]@{ t = 2500; tree = @(10, 11); rows = @([pscustomobject]@{ pid = 10; engtype = "Copy"; v = 4.0 }, [pscustomobject]@{ pid = 99; engtype = "3D"; v = 20.0 }) },
  [pscustomobject]@{ t = 5000; tree = @(10, 11); rows = @([pscustomobject]@{ pid = 10; engtype = "3D"; v = 99.0 }) }    # depois da fase
)
Add-GpuToPhase $phase $samples
$g = $phase.gpu_counters
Check "conta só as 2 amostras dentro da fase" ($g.samples_in_phase -eq 2)
Check "app 3D = (10+5 + 0)/2 = 7.5" ($g.app_tree_percent_by_engine["3D"] -eq 7.5)
Check "app Copy = (0 + 4)/2 = 2" ($g.app_tree_percent_by_engine["Copy"] -eq 2)
Check "todos os processos 3D = (55 + 20)/2 = 37.5" ($g.all_processes_percent_by_engine["3D"] -eq 37.5)
Check "pid fora da árvore não entra no app" ($g.app_tree_percent_by_engine["3D"] -lt $g.all_processes_percent_by_engine["3D"])

# --- Sem amostras: não deve lançar nem criar campo
$empty = [pscustomobject]@{ name = "x"; started_utc_ms = 1; ended_utc_ms = 2 }
Add-GpuToPhase $empty @()
Check "sem amostras não cria gpu_counters" ($null -eq $empty.PSObject.Properties["gpu_counters"])

if ($fail -gt 0) { Write-Host "$fail verificação(ões) falharam" -ForegroundColor Red; exit 1 }
Write-Host "selftest OK"
