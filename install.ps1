param(
    [string]$Version = $env:DEKAN_VERSION,
    [switch]$PreRelease = ($env:DEKAN_PRERELEASE -eq '1'),
    [switch]$NoLaunch = ($env:DEKAN_NO_LAUNCH -eq '1')
)

function Install-Dekan {
    param([string]$Version, [switch]$PreRelease, [switch]$NoLaunch)

    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $repo = 'chrisssst/Dekan'
    $api = "https://api.github.com/repos/$repo/releases"
    $headers = @{ 'User-Agent' = 'dekan-install'; 'Accept' = 'application/vnd.github+json' }
    $download = @{ 'User-Agent' = 'dekan-install' }

    if (-not [Environment]::Is64BitOperatingSystem) {
        throw 'Dekan needs 64-bit Windows 10 or 11.'
    }

    function Invoke-WithRetry([scriptblock]$Action, [string]$What) {
        for ($attempt = 1; ; $attempt++) {
            try { return & $Action }
            catch {
                if ($attempt -ge 3) { throw "Could not $What after $attempt attempts: $($_.Exception.Message)" }
                Start-Sleep -Seconds (2 * $attempt)
            }
        }
    }

    $release = Invoke-WithRetry -What 'read the release list from GitHub' -Action {
        if ($Version) {
            $tag = if ($Version.StartsWith('v')) { $Version } else { "v$Version" }
            Invoke-RestMethod -Uri "$api/tags/$tag" -Headers $headers -TimeoutSec 30 -UseBasicParsing
        } elseif ($PreRelease) {
            (Invoke-RestMethod -Uri "$api`?per_page=10" -Headers $headers -TimeoutSec 30 -UseBasicParsing) |
                Where-Object { -not $_.draft } | Select-Object -First 1
        } else {
            Invoke-RestMethod -Uri "$api/latest" -Headers $headers -TimeoutSec 30 -UseBasicParsing
        }
    }
    if (-not $release) { throw 'No published Dekan release was found.' }

    $setup = $release.assets | Where-Object { $_.name -match '^Dekan-Setup-.+-x64\.exe$' } | Select-Object -First 1
    $sums = $release.assets | Where-Object { $_.name -eq 'SHA256SUMS' } | Select-Object -First 1
    if (-not $setup -or -not $sums) {
        throw "Release $($release.tag_name) has no installer or SHA256SUMS."
    }
    Write-Host "Dekan $($release.tag_name): downloading $($setup.name)"

    $work = Join-Path ([IO.Path]::GetTempPath()) ("dekan-install-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
        $setupPath = Join-Path $work $setup.name
        $sumsPath = Join-Path $work 'SHA256SUMS'
        Invoke-WithRetry -What "download $($setup.name)" -Action {
            Invoke-WebRequest -Uri $setup.browser_download_url -OutFile $setupPath -Headers $download -TimeoutSec 600 -UseBasicParsing
        } | Out-Null
        Invoke-WithRetry -What 'download SHA256SUMS' -Action {
            Invoke-WebRequest -Uri $sums.browser_download_url -OutFile $sumsPath -Headers $download -TimeoutSec 60 -UseBasicParsing
        } | Out-Null

        $expected = Get-Content $sumsPath | ForEach-Object {
            $parts = $_.Trim() -split '\s+\*?', 2
            if ($parts.Count -eq 2 -and $parts[1] -eq $setup.name) { $parts[0].ToLowerInvariant() }
        } | Select-Object -First 1
        if (-not $expected) { throw "SHA256SUMS does not list $($setup.name)." }
        $actual = (Get-FileHash -Path $setupPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) {
            throw "Checksum mismatch for $($setup.name): expected $expected, got $actual. Nothing was installed."
        }
        Write-Host "Checksum verified: $actual"

        $signature = Get-AuthenticodeSignature -FilePath $setupPath
        switch ($signature.Status) {
            'Valid' { Write-Host "Signed by $($signature.SignerCertificate.Subject)" }
            'NotSigned' { Write-Warning 'This release is not code-signed; Windows SmartScreen may warn about it.' }
            default { throw "The installer signature is $($signature.Status). Nothing was installed." }
        }

        $elevated = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
            [Security.Principal.WindowsBuiltInRole]::Administrator)
        $setupArgs = '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /CLOSEAPPLICATIONS'
        Write-Host 'Installing; Windows asks for administrator permission once, for the copy into Program Files.'
        $process = if ($elevated) {
            Start-Process -FilePath $setupPath -ArgumentList $setupArgs -Wait -PassThru
        } else {
            Start-Process -FilePath $setupPath -ArgumentList $setupArgs -Verb RunAs -Wait -PassThru
        }
        if ($process.ExitCode -ne 0) {
            throw "The installer exited with code $($process.ExitCode)."
        }
    } finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }

    $uninstallKey = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{D387A5B1-8C56-4D2A-94B8-975DE11C6B45}_is1'
    $installDir = (Get-ItemProperty -Path $uninstallKey -ErrorAction SilentlyContinue).InstallLocation
    if (-not $installDir) { $installDir = Join-Path $env:ProgramFiles 'Dekan' }
    $exe = Join-Path $installDir 'dekan.exe'
    if (-not (Test-Path -LiteralPath $exe)) { throw "Dekan was not found at $exe after the install." }
    Write-Host "Dekan $($release.tag_name) installed in $installDir"

    if ($NoLaunch) { return }
    if ($elevated) {
        Write-Host 'This terminal runs as administrator, so Dekan is not started from it. Open Dekan from the Start menu.'
        return
    }
    Start-Process -FilePath $exe
    Write-Host 'Dekan started in the system tray. On first start it offers to download and verify the injector.'
}

Install-Dekan -Version $Version -PreRelease:$PreRelease -NoLaunch:$NoLaunch
