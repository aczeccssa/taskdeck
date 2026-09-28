[CmdletBinding()]
param(
    [string]$Version = $env:TASKDECK_VERSION,
    [string]$InstallDir = $env:TASKDECK_INSTALL_DIR,
    [string]$ReleasesUrl = $env:TASKDECK_RELEASES_URL,
    [string]$ReleaseApiUrl = $env:TASKDECK_RELEASE_API_URL
)

$ErrorActionPreference = 'Stop'
$explicitInstallDir = -not [string]::IsNullOrWhiteSpace($InstallDir)
$repository = if ($env:TASKDECK_REPOSITORY) { $env:TASKDECK_REPOSITORY } else { 'aczeccssa/taskdeck' }
if (-not $ReleasesUrl) { $ReleasesUrl = "https://github.com/$repository/releases" }
if (-not $ReleaseApiUrl) { $ReleaseApiUrl = "https://api.github.com/repos/$repository/releases/latest" }

function Get-TaskdeckTarget {
    $architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    switch ($architecture) {
        'X64' { return 'x86_64-pc-windows-msvc' }
        'Arm64' { return 'aarch64-pc-windows-msvc' }
        default { throw "Unsupported Windows CPU architecture: $architecture" }
    }
}

if (-not $Version) {
    $release = Invoke-RestMethod -Uri $ReleaseApiUrl -Headers @{ 'User-Agent' = 'taskdeck-installer' }
    $Version = [string]$release.tag_name
}
if ($Version -notmatch '^v\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') {
    throw "Invalid Taskdeck release version: $Version"
}

$target = Get-TaskdeckTarget
$assetName = "taskdeck-$Version-$target.zip"
$downloadUrl = "$($ReleasesUrl.TrimEnd('/'))/download/$Version"
$temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) "taskdeck-install-$([Guid]::NewGuid().ToString('N'))"
$null = New-Item -ItemType Directory -Path $temporaryRoot
$stagedPath = $null

try {
    $archivePath = Join-Path $temporaryRoot $assetName
    $checksumsPath = Join-Path $temporaryRoot 'SHA256SUMS'
    $extractPath = Join-Path $temporaryRoot 'extracted'
    Write-Host "Downloading Taskdeck $Version for $target..."
    Invoke-WebRequest -Uri "$downloadUrl/$assetName" -OutFile $archivePath -TimeoutSec 120
    Invoke-WebRequest -Uri "$downloadUrl/SHA256SUMS" -OutFile $checksumsPath -TimeoutSec 30

    $checksumPattern = '^([0-9a-fA-F]{64})\s+\*?' + [regex]::Escape($assetName) + '$'
    $expectedHash = $null
    foreach ($line in Get-Content -LiteralPath $checksumsPath) {
        $match = [regex]::Match($line, $checksumPattern)
        if ($match.Success) {
            $expectedHash = $match.Groups[1].Value
            break
        }
    }
    if (-not $expectedHash) { throw "Release checksum is missing or invalid for $assetName" }
    $actualHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash
    if (-not [string]::Equals($actualHash, $expectedHash, [StringComparison]::OrdinalIgnoreCase)) {
        throw "SHA256 checksum mismatch for $assetName"
    }

    Expand-Archive -LiteralPath $archivePath -DestinationPath $extractPath
    $binary = Get-ChildItem -LiteralPath $extractPath -Filter 'taskdeck.exe' -File -Recurse | Select-Object -First 1
    if (-not $binary) { throw 'Release archive does not contain taskdeck.exe' }

    if (-not $InstallDir) {
        $cargoRoot = if ($env:CARGO_INSTALL_ROOT) { $env:CARGO_INSTALL_ROOT }
            elseif ($env:CARGO_HOME) { $env:CARGO_HOME }
            else { Join-Path $HOME '.cargo' }
        $InstallDir = Join-Path $cargoRoot 'bin'
    }
    $InstallDir = [IO.Path]::GetFullPath($InstallDir)
    $null = New-Item -ItemType Directory -Force -Path $InstallDir
    $targetPath = Join-Path $InstallDir 'taskdeck.exe'
    $stagedPath = Join-Path $InstallDir ".taskdeck.new.$PID.exe"
    Copy-Item -LiteralPath $binary.FullName -Destination $stagedPath -Force

    if ($env:OS -eq 'Windows_NT' -and (Test-Path -LiteralPath $targetPath)) {
        try { & $targetPath shutdown *> $null } catch { }
        Start-Sleep -Milliseconds 1200
        try {
            Get-Process -Name taskdeck -ErrorAction SilentlyContinue |
                Where-Object { $_.Path -and [IO.Path]::GetFullPath($_.Path) -eq $targetPath } |
                Stop-Process -Force -ErrorAction SilentlyContinue
        } catch { }
    }

    Move-Item -LiteralPath $stagedPath -Destination $targetPath -Force
    $stagedPath = $null

    if ($env:OS -eq 'Windows_NT' -and -not $explicitInstallDir) {
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $pathEntries = @($userPath -split ';' | Where-Object { $_ })
        if (-not ($pathEntries | Where-Object { [string]::Equals($_.TrimEnd('\'), $InstallDir.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase) })) {
            [Environment]::SetEnvironmentVariable('Path', (($pathEntries + $InstallDir) -join ';'), 'User')
        }
        if (($env:Path -split ';') -notcontains $InstallDir) { $env:Path = "$InstallDir;$env:Path" }
    }

    Write-Host "Taskdeck $Version installed at $targetPath"
    if (($env:Path -split [IO.Path]::PathSeparator) -notcontains $InstallDir) {
        Write-Host "Add this directory to PATH to run taskdeck: $InstallDir"
    }
}
finally {
    if ($stagedPath -and (Test-Path -LiteralPath $stagedPath)) { Remove-Item -LiteralPath $stagedPath -Force }
    if (Test-Path -LiteralPath $temporaryRoot) { Remove-Item -LiteralPath $temporaryRoot -Recurse -Force }
}
