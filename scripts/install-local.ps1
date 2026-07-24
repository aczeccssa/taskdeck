[CmdletBinding()]
param(
    [switch]$Sync
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot

function Require-Command {
    param([Parameter(Mandatory)][string]$Name)

    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "$Name is required but was not found in PATH"
    }
}

if ($Sync) {
    Require-Command git
    $insideWorktree = git -C $repoRoot rev-parse --is-inside-work-tree 2>$null
    if ($LASTEXITCODE -ne 0 -or $insideWorktree -ne 'true') {
        throw "$repoRoot is not a Git worktree"
    }
    if (git -C $repoRoot status --porcelain) {
        throw 'Git sync requires a clean worktree; commit or stash local changes first'
    }
    $upstream = git -C $repoRoot rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $upstream) {
        throw 'current branch has no upstream; configure one before using -Sync'
    }
    $remote = ($upstream -split '/', 2)[0]
    if ($remote -eq $upstream) {
        throw "could not determine the remote from upstream '$upstream'"
    }
    Write-Host "Syncing Git worktree from $upstream..."
    git -C $repoRoot fetch --prune $remote
    if ($LASTEXITCODE -ne 0) {
        throw 'git fetch failed'
    }
    git -C $repoRoot merge --ff-only $upstream
    if ($LASTEXITCODE -ne 0) {
        throw 'git fast-forward merge failed'
    }
}

Require-Command cargo
$taskdeck = Get-Command taskdeck.exe -ErrorAction SilentlyContinue
if ($taskdeck) {
    & $taskdeck.Source shutdown *> $null
    if ($LASTEXITCODE -eq 0) {
        Write-Host 'Stopped the running Taskdeck daemon.'
        Start-Sleep -Milliseconds 1200
    }
    else {
        Write-Host 'No running Taskdeck daemon needed to be stopped.'
    }
    $targetPath = [IO.Path]::GetFullPath($taskdeck.Source)
    Get-Process taskdeck -ErrorAction SilentlyContinue |
        Where-Object { [IO.Path]::GetFullPath($_.Path) -eq $targetPath } |
        Stop-Process -Force -ErrorAction SilentlyContinue
}
else {
    Write-Host 'No running Taskdeck daemon needed to be stopped.'
}

Write-Host "Installing Taskdeck for Windows from $repoRoot..."
cargo install --locked --path $repoRoot --force
if ($LASTEXITCODE -ne 0) {
    throw 'cargo install failed'
}

$cargoRoot = if ($env:CARGO_INSTALL_ROOT) {
    $env:CARGO_INSTALL_ROOT
}
elseif ($env:CARGO_HOME) {
    $env:CARGO_HOME
}
else {
    Join-Path $env:USERPROFILE '.cargo'
}
$installDir = Join-Path $cargoRoot 'bin'
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$pathEntries = @($userPath -split ';' | Where-Object { $_ })
if ($pathEntries -notcontains $installDir) {
    [Environment]::SetEnvironmentVariable('Path', (($pathEntries + $installDir) -join ';'), 'User')
}

Write-Host 'Local installation completed.'
