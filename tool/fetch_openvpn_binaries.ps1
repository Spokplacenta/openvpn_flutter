# Downloads official OpenVPN Windows MSIs (amd64 + arm64),
# extracts runtime binaries and tapctl, stages them as Flutter assets
# under assets/openvpn/windows/<x64|arm64>/.
#
# OpenVPN 2.7 uses tapctl.exe to install/create ovpn-dco and TAP adapters.
#
# Usage: powershell -ExecutionPolicy Bypass -File tool/fetch_openvpn_binaries.ps1 -Version 2.7.5
param(
    [string]$Version = "2.7.5",
    [string[]]$InstallerTokens = @("I001", "I002")
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$repoRoot = Split-Path -Parent $PSScriptRoot
$assetsRoot = Join-Path $repoRoot "assets\openvpn\windows"
$work = Join-Path $env:TEMP ("ovpn_fetch_" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $work | Out-Null

$archMap = @{ "amd64" = "x64"; "arm64" = "arm64" }

function Download-First {
    param([string[]]$Urls, [string]$OutFile)
    foreach ($u in $Urls) {
        try {
            Invoke-WebRequest -Uri $u -OutFile $OutFile -UseBasicParsing -TimeoutSec 120
            Write-Output "  downloaded: $u"
            return $true
        }
        catch {
            Write-Output "  miss: $u ($($_.Exception.Message))"
        }
    }
    return $false
}

function Find-One {
    param([string]$Root, [string]$Pattern)
    $hit = Get-ChildItem -Path $Root -Recurse -File -Filter $Pattern -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($hit) { return $hit.FullName }
    return $null
}

foreach ($msiArch in $archMap.Keys) {
    $sub = $archMap[$msiArch]
    $destDir = Join-Path $assetsRoot $sub
    New-Item -ItemType Directory -Force -Path $destDir | Out-Null
    Write-Output "== OpenVPN $Version $msiArch -> $sub =="

    $urls = @()
    foreach ($tok in $InstallerTokens) {
        $urls += "https://build.openvpn.net/downloads/releases/OpenVPN-$Version-$tok-$msiArch.msi"
    }
    $msi = Join-Path $work "OpenVPN-$Version-$msiArch.msi"
    if (-not (Download-First -Urls $urls -OutFile $msi)) {
        throw "Unable to download OpenVPN MSI for $msiArch (version $Version)"
    }

    $extract = Join-Path $work "extract_$msiArch"
    New-Item -ItemType Directory -Force -Path $extract | Out-Null
    $log = Join-Path $work "msi_$msiArch.log"
    $p = Start-Process msiexec.exe -ArgumentList @("/a", "`"$msi`"", "/qn", "TARGETDIR=`"$extract`"", "/l*v", "`"$log`"") -Wait -PassThru
    if ($p.ExitCode -ne 0) { throw "msiexec extraction failed for $msiArch (exit $($p.ExitCode))" }

    $copies = @(
        @{ pat = "openvpn.exe";            dest = "openvpn.exe.bin" },
        @{ pat = "openvpnserv.exe";        dest = "openvpnserv.exe.bin" },
        @{ pat = "openvpnservmsg.dll";     dest = "openvpnservmsg.dll.bin" },
        @{ pat = "tapctl.exe";             dest = "tapctl.exe.bin" },
        @{ pat = "libpkcs11-helper-1.dll"; dest = "libpkcs11-helper-1.dll" },
        @{ pat = "libcrypto-3-*.dll";      dest = $null },
        @{ pat = "libssl-3-*.dll";         dest = $null }
    )

    foreach ($c in $copies) {
        $src = Find-One -Root $extract -Pattern $c.pat
        if (-not $src) { throw "Missing '$($c.pat)' in extracted $msiArch MSI" }
        $destName = $c.dest
        if (-not $destName) { $destName = Split-Path $src -Leaf }
        Copy-Item -Path $src -Destination (Join-Path $destDir $destName) -Force
        Write-Output "  copied: $(Split-Path $src -Leaf) -> $destName"
    }
}

Write-Output ""
Write-Output "===== SHA256 (fill into WindowsBinaryManager) ====="
foreach ($msiArch in $archMap.Keys) {
    $sub = $archMap[$msiArch]
    $destDir = Join-Path $assetsRoot $sub
    Write-Output "--- $sub ---"
    Get-ChildItem -Path $destDir -File | ForEach-Object {
        $h = (Get-FileHash -Algorithm SHA256 -Path $_.FullName).Hash
        Write-Output ("  {0}  {1}  ({2} bytes)" -f $_.Name, $h, $_.Length)
    }
}

Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
Write-Output "DONE"
