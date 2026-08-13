# Install conversation-handoff from GitHub Releases (no Rust required).
$ErrorActionPreference = "Stop"

$Repo = if ($env:CONVERSATION_HANDOFF_REPO) { $env:CONVERSATION_HANDOFF_REPO } else { "nblotti/conversation-handoff" }
$InstallDir = if ($env:CONVERSATION_HANDOFF_BIN) { $env:CONVERSATION_HANDOFF_BIN } else { Join-Path $env:LOCALAPPDATA "conversation-handoff" }
$Name = "conversation-handoff.exe"
$Asset = "conversation-handoff-windows-x86_64.exe"
$Url = "https://github.com/$Repo/releases/latest/download/$Asset"

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$Dest = Join-Path $InstallDir $Name
Write-Host "Downloading $Url"
Invoke-WebRequest -Uri $Url -OutFile $Dest -UseBasicParsing

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", "User")
    $env:Path = "$env:Path;$InstallDir"
    Write-Host "Added $InstallDir to your user PATH. Open a new terminal if the command is not found."
}

& $Dest install --write-instructions
Write-Host "Done. Start a new Claude Code or Codex session and approve the tools."
