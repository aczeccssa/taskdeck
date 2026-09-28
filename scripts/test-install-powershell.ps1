$ErrorActionPreference = 'Stop'

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

$installerPath = Join-Path $PSScriptRoot 'install.ps1'

# The fixture server needs a working Python 3 interpreter. Prefer a real installation and
# keep the Microsoft Store App Execution Alias as a last resort: that stub lives under
# WindowsApps, resolves through Get-Command, but cannot actually be launched.
$python = $null
$storeStub = $null
foreach ($candidate in 'python3', 'python', 'py') {
    $command = Get-Command $candidate -ErrorAction SilentlyContinue
    if (-not $command) { continue }
    if ($command.Source -like '*\WindowsApps\*') {
        if (-not $storeStub) { $storeStub = $command.Source }
        continue
    }
    $python = $command.Source
    break
}
if (-not $python) { $python = $storeStub }
if (-not $python) { throw 'A Python 3 interpreter is required to serve the local release fixture' }

$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
$target = switch ($architecture) {
    'X64' { 'x86_64-pc-windows-msvc' }
    'Arm64' { 'aarch64-pc-windows-msvc' }
    default { throw "This smoke test needs an x64 or arm64 host; got $architecture" }
}
$version = 'v0.1.0'
$temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) "taskdeck-powershell-smoke-$([Guid]::NewGuid().ToString('N'))"
$fixtureRoot = Join-Path $temporaryRoot 'fixture'
$downloadRoot = Join-Path $fixtureRoot "releases/download/$version"
$payloadRoot = Join-Path $temporaryRoot 'payload'
$null = New-Item -ItemType Directory -Force -Path $downloadRoot, $payloadRoot
$archiveName = "taskdeck-$version-$target.zip"
$archivePath = Join-Path $downloadRoot $archiveName
$stubPath = Join-Path $payloadRoot 'taskdeck.exe'
$serverOutput = Join-Path $temporaryRoot 'server.out'
$serverError = Join-Path $temporaryRoot 'server.err'
$serverProcess = $null
$oldEnvironment = @{}

try {
    Set-Content -LiteralPath $stubPath -Value 'Windows release executable fixture' -NoNewline -Encoding Ascii
    Compress-Archive -LiteralPath $stubPath -DestinationPath $archivePath
    $archiveHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    Set-Content -LiteralPath (Join-Path $downloadRoot 'SHA256SUMS') -Value "$archiveHash  $archiveName" -Encoding Ascii
    Copy-Item -LiteralPath $installerPath -Destination (Join-Path $fixtureRoot 'install.ps1')
    Set-Content -LiteralPath (Join-Path $fixtureRoot 'latest.json') -Value (@{ tag_name = $version } | ConvertTo-Json) -Encoding Ascii

    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = ([Net.IPEndPoint]$listener.LocalEndpoint).Port
    $listener.Stop()
    # Redirect stdin as well. A process that inherits the caller's stdin keeps that
    # handle open, which stops a wrapping ssh session or CI job from ever exiting.
    $stdinNull = Join-Path $temporaryRoot 'stdin.null'
    Set-Content -LiteralPath $stdinNull -Value '' -NoNewline -Encoding Ascii
    $serverProcess = Start-Process -FilePath $python -ArgumentList @('-m', 'http.server', "$port", '--bind', '127.0.0.1', '--directory', $fixtureRoot) `
        -PassThru -RedirectStandardOutput $serverOutput -RedirectStandardError $serverError -RedirectStandardInput $stdinNull
    $serverUrl = "http://127.0.0.1:$port"
    $serverReady = $false
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        try {
            # -UseBasicParsing skips the Internet Explorer engine, which is unavailable in a
            # headless session and would make every probe fail even with the server running.
            $null = Invoke-WebRequest -Uri "$serverUrl/latest.json" -TimeoutSec 1 -UseBasicParsing
            $serverReady = $true
            break
        } catch { Start-Sleep -Milliseconds 200 }
    }
    Assert-True $serverReady 'Local release fixture server did not start'

    $environment = @{
        TASKDECK_RELEASE_API_URL = "$serverUrl/latest.json"
        TASKDECK_RELEASES_URL = "$serverUrl/releases"
        TASKDECK_INSTALL_DIR = (Join-Path $temporaryRoot 'installed')
    }
    foreach ($name in $environment.Keys) {
        $oldEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        [Environment]::SetEnvironmentVariable($name, $environment[$name], 'Process')
    }

    Write-Host 'Testing the PowerShell install pipeline against a local release fixture...'
    Invoke-RestMethod -Uri "$serverUrl/install.ps1" | Invoke-Expression
    $installedBinary = Join-Path $environment.TASKDECK_INSTALL_DIR 'taskdeck.exe'
    Assert-True (Test-Path -LiteralPath $installedBinary) 'Installer did not write taskdeck.exe'
    $installedHash = (Get-FileHash -LiteralPath $installedBinary -Algorithm SHA256).Hash
    $stubHash = (Get-FileHash -LiteralPath $stubPath -Algorithm SHA256).Hash
    Assert-True ($installedHash -eq $stubHash) 'Installed executable does not match the verified archive'

    Set-Content -LiteralPath (Join-Path $downloadRoot 'SHA256SUMS') -Value (('0' * 64) + "  $archiveName") -Encoding Ascii
    $environment.TASKDECK_INSTALL_DIR = Join-Path $temporaryRoot 'rejected'
    [Environment]::SetEnvironmentVariable('TASKDECK_INSTALL_DIR', $environment.TASKDECK_INSTALL_DIR, 'Process')
    $rejectedBadChecksum = $false
    try {
        Invoke-RestMethod -Uri "$serverUrl/install.ps1" | Invoke-Expression
    } catch {
        $rejectedBadChecksum = $_.Exception.Message -match 'SHA256 checksum mismatch'
    }
    Assert-True $rejectedBadChecksum 'Installer did not reject a bad SHA256 checksum'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $environment.TASKDECK_INSTALL_DIR 'taskdeck.exe'))) 'Bad checksum left an executable installed'
    Write-Host 'PowerShell installer smoke passed; all release traffic stayed on localhost.'
}
finally {
    foreach ($name in $oldEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $oldEnvironment[$name], 'Process')
    }
    # /T kills the whole tree. `Stop-Process -Id` alone leaves the interpreter running
    # whenever it was resolved through a launcher, and that orphan keeps the temp files
    # open, so the recursive delete below would fail with an IOException.
    if ($serverProcess) { & taskkill /PID $serverProcess.Id /T /F *> $null }
    if (Test-Path -LiteralPath $temporaryRoot) { Remove-Item -LiteralPath $temporaryRoot -Recurse -Force }
}
