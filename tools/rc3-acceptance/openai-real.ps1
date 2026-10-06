<#
.SYNOPSIS
  RC3 - real OpenAI acceptance harness (external gate; runs on YOUR Windows machine).

.DESCRIPTION
  Uses the OpenAI API key that is ALREADY stored in the Windows Credential Manager by the app
  (reference capia/provider/openai). This script never reads, prints, logs or writes the key: the
  capia-devserver process (started with --os-vault) reads it inside Rust and redacts it everywhere.
  Official OpenAI endpoint only (no base_url override).

  Steps: connect (import models + probe + automatic Brain Profile) -> chat "what can you do?" ->
  simple edit (preview -> approval -> apply -> undo, exact) -> optional vision probe ->
  speech-to-text with whisper-1, separately -> secret scan of every captured output.

  No key in the Credential Manager  =>  status "pending_external" (exit code 3). That is NEVER a
  success: the external gate stays open. Exit codes: 0 all steps ok, 1 a step failed, 3 pending_external.

.PARAMETER DevServer
  Path to capia-devserver.exe (CI artifact "capia-rc3-live-harness" or built with
  `cargo build --release -p capia-devserver`).
.PARAMETER FfmpegDir
  Folder with ffmpeg.exe/ffprobe.exe (the installed app ships them). Needed only for the STT step.
.PARAMETER Vision
  Also report the vision capability probe.
.PARAMETER Out
  Output folder for summary.json and logs (default: .\rc3-openai-real-out).

.EXAMPLE
  .\openai-real.ps1 -DevServer .\capia-devserver.exe -FfmpegDir "C:\Program Files\CapIA\resources\ffmpeg"
#>
param(
  [string]$DevServer = ".\capia-devserver.exe",
  [string]$FfmpegDir = "",
  [switch]$Vision,
  [int]$Port = 5391,
  [string]$Out = ".\rc3-openai-real-out"
)

$ErrorActionPreference = "Stop"
$script:steps = New-Object System.Collections.ArrayList
$script:base = "http://127.0.0.1:$Port"
$script:secretPattern = "sk-[A-Za-z0-9_\-]{20,}"

function Add-Step([string]$name, [string]$status, [string]$detail, [int]$ms = 0) {
  [void]$script:steps.Add([ordered]@{ name = $name; status = $status; detail = $detail; ms = $ms })
  $tag = $status.ToUpper()
  Write-Host ("[{0}] {1} - {2}" -f $tag, $name, $detail)
}

# POST /api/<method>; returns @{ ok; body; code; message }
function Invoke-Api([string]$method, $payload = @{}) {
  $json = ConvertTo-Json -InputObject $payload -Depth 20 -Compress
  $req = [System.Net.WebRequest]::Create("$($script:base)/api/$method")
  $req.Method = "POST"
  $req.ContentType = "application/json"
  $req.Timeout = 180000
  $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
  $req.ContentLength = $bytes.Length
  $s = $req.GetRequestStream(); $s.Write($bytes, 0, $bytes.Length); $s.Close()
  try {
    $resp = $req.GetResponse()
    $text = (New-Object System.IO.StreamReader($resp.GetResponseStream())).ReadToEnd()
    $resp.Close()
    return @{ ok = $true; body = ($text | ConvertFrom-Json); code = ""; message = "" }
  } catch [System.Net.WebException] {
    $text = ""
    if ($_.Exception.Response) {
      $text = (New-Object System.IO.StreamReader($_.Exception.Response.GetResponseStream())).ReadToEnd()
    }
    $code = "HTTP_ERROR"; $msg = $_.Exception.Message
    try { $e = $text | ConvertFrom-Json; if ($e.code) { $code = [string]$e.code }; if ($e.message) { $msg = [string]$e.message } } catch {}
    return @{ ok = $false; body = $null; code = $code; message = $msg }
  }
}

function Wait-Task([string]$taskId, [int]$timeoutSec = 180) {
  $deadline = (Get-Date).AddSeconds($timeoutSec)
  while ((Get-Date) -lt $deadline) {
    $r = Invoke-Api "ai.task.get" @{ task_id = $taskId }
    if (-not $r.ok) { return $r }
    if ($r.body.state -ne "running") { return $r }
    Start-Sleep -Milliseconds 400
  }
  return @{ ok = $false; body = $null; code = "TIMEOUT"; message = "the task did not finish in time" }
}

function Finish([string]$status, [int]$exitCode) {
  # secret scan over everything we captured (the key itself is unknown to this script)
  $leak = $false
  foreach ($f in @("devserver.out.log", "devserver.err.log")) {
    $p = Join-Path $Out $f
    if ((Test-Path $p) -and ((Get-Content $p -Raw) -match $script:secretPattern)) { $leak = $true }
  }
  $summaryText = ConvertTo-Json -InputObject $script:steps -Depth 6
  if ($summaryText -match $script:secretPattern) { $leak = $true }
  if ($leak) {
    Add-Step "secret_scan" "failed" "an API-key-shaped string was found in the captured output" 0
    $status = "failed"; $exitCode = 1
  } else {
    Add-Step "secret_scan" "ok" "no API-key-shaped string in the logs or in the summary" 0
  }
  $summary = [ordered]@{
    harness = "rc3-openai-real"
    status = $status
    note = "pending_external means the external gate is still OPEN (no key / not run); it is never a success"
    generated = (Get-Date).ToString("o")
    steps = $script:steps
  }
  $summary | ConvertTo-Json -Depth 8 | Set-Content -Path (Join-Path $Out "summary.json") -Encoding UTF8
  Write-Host ""
  Write-Host "RESULT: $status  (summary: $(Join-Path $Out 'summary.json'))"
  if ($script:proc -and -not $script:proc.HasExited) { try { $script:proc.Kill() } catch {} }
  exit $exitCode
}

New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path $Out).Path
if (-not (Test-Path $DevServer)) {
  Add-Step "start" "failed" "capia-devserver.exe not found at '$DevServer' (see -DevServer)"
  Finish "failed" 1
}
if ($FfmpegDir -ne "") {
  $env:CAPIA_FFMPEG = Join-Path $FfmpegDir "ffmpeg.exe"
  $env:CAPIA_FFPROBE = Join-Path $FfmpegDir "ffprobe.exe"
}
$env:CAPIA_AI_APPDB = Join-Path $Out "ai-app.db"
if (Test-Path $env:CAPIA_AI_APPDB) { Remove-Item $env:CAPIA_AI_APPDB -Force }

# ---- 0. start the engine with the REAL OS credential store (never the in-memory one)
$script:proc = Start-Process -FilePath $DevServer `
  -ArgumentList @("--port", "$Port", "--os-vault", "--static", $Out) `
  -RedirectStandardOutput (Join-Path $Out "devserver.out.log") `
  -RedirectStandardError (Join-Path $Out "devserver.err.log") `
  -PassThru -WindowStyle Hidden
$up = $false
for ($i = 0; $i -lt 100; $i++) {
  Start-Sleep -Milliseconds 100
  if ($script:proc.HasExited) { break }
  $r = Invoke-Api "ai.status"
  if ($r.ok) { $up = $true; break }
}
if (-not $up) {
  Add-Step "start" "failed" "the engine did not start (see devserver.err.log); --os-vault needs the Windows Credential Manager"
  Finish "failed" 1
}
Add-Step "start" "ok" "engine started with the operating-system credential store"

# ---- 1. connect with the key already stored by the app (official OpenAI, no base_url override)
$t0 = Get-Date
$c = Invoke-Api "ai.connect" @{ preset = "openai"; use_stored = $true }
if (-not $c.ok) {
  if ($c.code -eq "NO_CREDENTIAL") {
    Add-Step "credential" "pending_external" "no OpenAI key in the Windows Credential Manager - connect OpenAI once in the app (AI > Connect), then run again"
    foreach ($n in @("connect", "import_models", "probe", "brain_profile", "chat", "edit_preview_approve_apply_undo", "vision", "stt_whisper_1")) {
      Add-Step $n "pending_external" "needs the stored key"
    }
    Finish "pending_external" 3
  }
  Add-Step "credential" "failed" "$($c.code): $($c.message)"
  Finish "failed" 1
}
Add-Step "credential" "ok" "a stored key exists (value never read by this script)"
$task = Wait-Task $c.body.task_id 240
$ms = [int]((Get-Date) - $t0).TotalMilliseconds
if (-not $task.ok -or $task.body.state -ne "done") {
  $why = if ($task.ok) { "$($task.body.error)" } else { "$($task.code): $($task.message)" }
  Add-Step "connect" "failed" $why $ms
  Finish "failed" 1
}
$res = $task.body.result
Add-Step "connect" "ok" "model $($res.model); official OpenAI" $ms
$models = (Invoke-Api "ai.status").body.models
Add-Step "import_models" $(if ($models.Count -gt 0) { "ok" } else { "failed" }) "$($models.Count) models imported"
Add-Step "probe" $(if ($res.probe.connected) { "ok" } else { "failed" }) "probe status: $($res.probe.status)"
Add-Step "brain_profile" $(if ($res.profile_created -and $res.active_profile -eq "auto") { "ok" } else { "failed" }) "automatic Brain Profile active: $($res.active_profile)"

# ---- 2. chat
$ask = "o que voc" + [char]0x00EA + " pode fazer?"
$t0 = Get-Date
$s = Invoke-Api "ai.assistant.send" @{ text = $ask; mode = "ask" }
if ($s.ok) {
  $w = Wait-Task $s.body.task_id 180
  $ms = [int]((Get-Date) - $t0).TotalMilliseconds
  $final = if ($w.ok) { [string]$w.body.result.final_text } else { "" }
  if ($w.ok -and $final.Length -gt 0) { Add-Step "chat" "ok" "answered ($($final.Length) chars)" $ms }
  else { Add-Step "chat" "failed" "no answer: $($w.code) $($w.message) $($w.body.error)" $ms }
} else { Add-Step "chat" "failed" "$($s.code): $($s.message)" }

# ---- 3. simple edit: preview -> approval -> apply -> undo (exact)
$proj = Join-Path $Out "live.capia"
if (Test-Path $proj) { Remove-Item $proj -Force }
[void](Invoke-Api "project.create" @{ path = $proj })
$frame = 23520000
$build = Invoke-Api "command.execute" @{ label = "harness setup"; commands = @(
    @{ operation_id = "h1"; type = "create_sequence"; id = "hs"; name = "Live"; frame_rate = "30"; width = 1080; height = 1920 },
    @{ operation_id = "h2"; type = "add_track"; sequence = "hs"; id = "ht"; kind = "visual" },
    @{ operation_id = "h3"; type = "insert_clip"; track = "ht"; start = 0;
       clip = @{ id = "hclip"; duration = (300 * $frame); content = @{ type = "solid"; color = "#336699" } } }
  ) }
if (-not $build.ok) {
  Add-Step "edit_preview_approve_apply_undo" "failed" "setup: $($build.code) $($build.message)"
} else {
  $before = (Invoke-Api "sequence.get" @{ sequence = "hs" }).body.clips.hclip | ConvertTo-Json -Depth 12 -Compress
  $t0 = Get-Date
  $e = Invoke-Api "ai.assistant.send" @{
    text = "remova os primeiros 2 segundos do clipe selecionado"; mode = "ask"
    selected_clips = @("hclip"); playhead_ticks = 0; sequence = "hs" }
  if (-not $e.ok) { Add-Step "edit_preview_approve_apply_undo" "failed" "$($e.code): $($e.message)" }
  else {
    $w = Wait-Task $e.body.task_id 240
    $rec = if ($w.ok) { $w.body.result } else { $null }
    if ($null -eq $rec -or $rec.status -ne "awaiting_approval" -or -not $rec.pending) {
      $st = if ($rec) { $rec.status } else { "$($w.code)" }
      Add-Step "edit_preview_approve_apply_undo" "failed" "the model did not propose an edit that waits for approval (status: $st)"
    } else {
      $unchanged = ((Invoke-Api "sequence.get" @{ sequence = "hs" }).body.clips.hclip | ConvertTo-Json -Depth 12 -Compress) -eq $before
      $ap = Invoke-Api "ai.assistant.approve" @{ task_id = $e.body.task_id; plan_token = $rec.pending.plan_token }
      $after = (Invoke-Api "sequence.get" @{ sequence = "hs" }).body.clips.hclip | ConvertTo-Json -Depth 12 -Compress
      $changed = $after -ne $before
      [void](Invoke-Api "command.undo")
      $restored = ((Invoke-Api "sequence.get" @{ sequence = "hs" }).body.clips.hclip | ConvertTo-Json -Depth 12 -Compress) -eq $before
      $ms = [int]((Get-Date) - $t0).TotalMilliseconds
      if ($unchanged -and $ap.ok -and $changed -and $restored) {
        Add-Step "edit_preview_approve_apply_undo" "ok" "proposal waited for approval, applied, undone back to the exact original" $ms
      } else {
        Add-Step "edit_preview_approve_apply_undo" "failed" "unchangedBeforeApproval=$unchanged approved=$($ap.ok) changedAfter=$changed restoredByUndo=$restored" $ms
      }
    }
  }
}

# ---- 4. vision (optional)
if ($Vision) {
  $v = $res.probe
  $text = ($v | ConvertTo-Json -Depth 8 -Compress)
  if ($text -match "vision") { Add-Step "vision" "ok" "vision capability reported by the probe (see summary of connect); no image was sent" }
  else { Add-Step "vision" "failed" "the probe did not report the vision capability" }
} else { Add-Step "vision" "skipped" "not requested (use -Vision)" }

# ---- 5. speech-to-text with whisper-1, separately from the chat model
if (-not $res.stt_endpoint_id) {
  Add-Step "stt_whisper_1" "failed" "the account lists no whisper-1 model"
} elseif ($FfmpegDir -eq "" -and -not (Get-Command ffprobe -ErrorAction SilentlyContinue)) {
  Add-Step "stt_whisper_1" "skipped" "FFmpeg not found (use -FfmpegDir); STT needs it to import the audio"
} else {
  try {
    Add-Type -AssemblyName System.Speech
    $wav = Join-Path $Out "speech.wav"
    $syn = New-Object System.Speech.Synthesis.SpeechSynthesizer
    $syn.SetOutputToWaveFile($wav)
    $syn.Speak("Hello, this is a short test of the transcription. Buy now.")
    $syn.Dispose()
    $imp = Invoke-Api "assets.import" @{ paths = @($wav) }
    $assetId = $null
    for ($i = 0; $i -lt 150 -and -not $assetId; $i++) {
      Start-Sleep -Milliseconds 200
      $evs = (Invoke-Api "events.poll").body.events
      foreach ($ev in $evs) { if ($ev.kind -eq "import_finalized") { $assetId = $ev.result.asset_id } }
    }
    if (-not $assetId) { throw "the audio import did not finish" }
    $t0 = Get-Date
    $tr = Invoke-Api "ai.transcribe" @{ asset_id = $assetId }
    if (-not $tr.ok) { throw "$($tr.code): $($tr.message)" }
    $w = Wait-Task $tr.body.task_id 240
    $ms = [int]((Get-Date) - $t0).TotalMilliseconds
    if ($w.ok -and $w.body.state -eq "done" -and $w.body.result.segments -gt 0) {
      Add-Step "stt_whisper_1" "ok" "model $($w.body.result.model): $($w.body.result.segments) segment(s)" $ms
    } else { Add-Step "stt_whisper_1" "failed" "transcription failed: $($w.body.error)" $ms }
  } catch { Add-Step "stt_whisper_1" "failed" "$($_.Exception.Message)" }
}

$failed = @($script:steps | Where-Object { $_.status -eq "failed" }).Count
if ($failed -gt 0) { Finish "failed" 1 } else { Finish "ok" 0 }
