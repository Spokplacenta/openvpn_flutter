import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/foundation.dart' show debugPrint;
import 'package:flutter/services.dart' show rootBundle;
import 'package:path/path.dart' as path;

import 'windows_binary_manager.dart';

/// Deploys [tapctl.exe] and creates ovpn-dco / TAP adapters on Windows.
///
/// OpenVPN 2.7 uses `tapctl create --hwid ovpn-dco` (default) for Win-DCO and
/// `tapctl create --hwid tap0901` for TAP fallback adapters.
class WindowsDriverManager {
  static const String tapCtlAssetName = 'tapctl.exe.bin';
  static const String tapCtlBinaryName = 'tapctl.exe';

  static List<String> get _assetBasePaths => [
        'packages/openvpn_flutter/assets/openvpn/windows/${WindowsBinaryManager.windowsArch}',
        'assets/openvpn/windows/${WindowsBinaryManager.windowsArch}',
      ];

  static Future<String> getMachineDirectory() async {
    return WindowsBinaryManager.getMachineRuntimeDirectory();
  }

  static Future<String> getStagingDirectory() async {
    final staging = await WindowsBinaryManager.getRuntimeDirectory();
    return staging.path;
  }

  static Future<String> getMachineTapCtlPath() async {
    final machineDir = await getMachineDirectory();
    return path.join(machineDir, tapCtlBinaryName);
  }

  static Future<String> getStagingTapCtlPath() async {
    final stagingDir = await getStagingDirectory();
    return path.join(stagingDir, tapCtlBinaryName);
  }

  static Future<bool> isDcoAdapterPresent() async {
    return _runPowerShellBool(
      "Get-NetAdapter -ErrorAction SilentlyContinue | "
      "Where-Object { \$_.InterfaceDescription -match "
      "'OpenVPN Data Channel|ovpn-dco|Data Channel Offload' } | "
      "Select-Object -First 1 | ForEach-Object { \$true }",
    );
  }

  static Future<bool> isTapAdapterPresent() async {
    return _runPowerShellBool(
      "Get-NetAdapter -ErrorAction SilentlyContinue | "
      "Where-Object { \$_.InterfaceDescription -match 'TAP-Windows|TAP-Windows6' } | "
      "Select-Object -First 1 | ForEach-Object { \$true }",
    );
  }

  static Future<bool> _runPowerShellBool(String script) async {
    if (!Platform.isWindows) return false;
    try {
      final result = await Process.run(
        'powershell.exe',
        ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', script],
        runInShell: true,
      );
      return (result.stdout as String).trim().toLowerCase().contains('true');
    } catch (_) {
      return false;
    }
  }

  /// Copies bundled tapctl.exe and optional driver packages to the staging directory.
  static Future<bool> ensureDriversDeployed({
    void Function(double progress)? onProgress,
  }) async {
    if (!Platform.isWindows) return true;
    onProgress?.call(0.1);

    final data = await _loadTapCtlAsset();
    if (data == null || data.lengthInBytes == 0) {
      debugPrint('⚠️ [OpenVPN] tapctl.exe asset missing in bundle');
      onProgress?.call(1.0);
      return false;
    }

    final stagingDir = await getStagingDirectory();
    final dest = path.join(stagingDir, tapCtlBinaryName);
    await File(dest).writeAsBytes(data.buffer.asUint8List(), flush: true);
    debugPrint('✅ [OpenVPN] tapctl deployed: $dest');

    onProgress?.call(0.5);
    await _deployBundledDriverAssets(stagingDir);
    onProgress?.call(1.0);
    return true;
  }

  static Future<void> _deployBundledDriverAssets(String stagingDir) async {
    final arch = WindowsBinaryManager.windowsArch;
    final platform = arch == 'arm64' ? 'arm64' : 'amd64';
    final driverFiles = <String>[
      'drivers/ovpn-dco/win10/ovpn-dco.inf',
      'drivers/ovpn-dco/win10/ovpn-dco.cat',
      'drivers/ovpn-dco/win10/ovpn-dco.sys',
      'drivers/ovpn-dco/win11/ovpn-dco.inf',
      'drivers/ovpn-dco/win11/ovpn-dco.cat',
      'drivers/ovpn-dco/win11/ovpn-dco.sys',
      'drivers/tap/$platform/win10/OemVista.inf',
      'drivers/tap/$platform/win10/tap0901.cat',
      'drivers/tap/$platform/win10/tap0901.sys',
    ];

    var copied = 0;
    for (final relative in driverFiles) {
      final bytes = await _loadDriverAsset(relative);
      if (bytes == null) continue;
      final out = path.join(stagingDir, relative);
      await Directory(path.dirname(out)).create(recursive: true);
      await File(out).writeAsBytes(bytes, flush: true);
      copied++;
    }
    if (copied > 0) {
      debugPrint('✅ [OpenVPN] $copied driver file(s) deployed under $stagingDir/drivers');
    } else {
      debugPrint(
        '⚠️ [OpenVPN] No bundled driver packages in app assets; '
        'run tool/fetch_openvpn_binaries.ps1 before building',
      );
    }
  }

  static Future<List<int>?> _loadDriverAsset(String relativePath) async {
    for (final base in _assetBasePaths) {
      final candidate = '$base/$relativePath';
      try {
        final data = await rootBundle.load(candidate);
        return data.buffer.asUint8List().toList();
      } catch (_) {}
    }
    return null;
  }

  static Future<ByteData?> _loadTapCtlAsset() async {
    for (final base in _assetBasePaths) {
      final candidate = '$base/$tapCtlAssetName';
      try {
        return await rootBundle.load(candidate);
      } catch (_) {}
    }
    return null;
  }

  /// PowerShell fragment for elevated adapter/driver setup via tapctl.
  ///
  /// Driver packages may be bundled under
  /// `assets/openvpn/windows/<arch>/drivers/` (installed with pnputil before
  /// tapctl create). Failures here must never abort OpenVPN service install.
  static String installDriversScriptBody({
    required String stagingDir,
    required String machineDir,
  }) {
    final staging = stagingDir.replaceAll("'", "''");
    final machine = machineDir.replaceAll("'", "''");
    return '''
function Invoke-NativeLogged {
  param(
    [string]\$Label,
    [string]\$FilePath,
    [string[]]\$ArgumentList
  )
  \$prev = \$ErrorActionPreference
  \$ErrorActionPreference = 'Continue'
  try {
    \$output = & \$FilePath @ArgumentList 2>&1 | Out-String
    \$exit = \$LASTEXITCODE
    if ([string]::IsNullOrWhiteSpace(\$output)) { \$output = '(no output)' }
    Log-Step "[LAVCONTROL] \$Label exit=\$exit output:`n\$output"
    return \$output
  } catch {
    Log-Step "[LAVCONTROL] \$Label threw: \$(\$_.Exception.Message)"
    return ''
  } finally {
    \$ErrorActionPreference = \$prev
  }
}

function Get-OvpnDcoOsFlavor {
  try {
    \$build = [int](Get-CimInstance Win32_OperatingSystem).BuildNumber
    if (\$build -ge 22000) { return 'win11' }
  } catch {}
  return 'win10'
}

function Install-DriverPackageIfPresent {
  param(
    [string]\$Label,
    [string]\$InfPath
  )
  if (-not (Test-Path -LiteralPath \$InfPath)) {
    Log-Step "[LAVCONTROL] \$Label driver package missing: \$InfPath"
    return \$false
  }
  Log-Step "[LAVCONTROL] Installing \$Label driver via pnputil: \$InfPath"
  Invoke-NativeLogged -Label "pnputil \$Label" -FilePath 'pnputil.exe' -ArgumentList @('/add-driver', \$InfPath, '/install') | Out-Null
  return \$true
}

function Invoke-TapCtlCreateWithRetry {
  param(
    [string]\$TapCtl,
    [string]\$Label,
    [string[]]\$ArgumentList,
    [int]\$MaxAttempts = 6,
    [int]\$DelaySeconds = 2
  )
  for (\$attempt = 1; \$attempt -le \$MaxAttempts; \$attempt++) {
    \$output = Invoke-NativeLogged -Label "\$Label attempt \$attempt/\$MaxAttempts" -FilePath \$TapCtl -ArgumentList \$ArgumentList
    if (\$output -match '\{[0-9a-fA-F-]{36}\}') {
      Log-Step "[LAVCONTROL] \$Label succeeded on attempt \$attempt"
      return \$true
    }
    if (\$output -notmatch 'SetupDiOpenDevRegKey failed|DinstallDevice failed|failed|error') {
      Log-Step "[LAVCONTROL] \$Label finished without GUID but no fatal pattern; stopping retries"
      return \$false
    }
    if (\$attempt -lt \$MaxAttempts) {
      Log-Step "[LAVCONTROL] \$Label retry in \$DelaySeconds s (SetupDiOpenDevRegKey / driver store not ready yet)"
      Start-Sleep -Seconds \$DelaySeconds
    }
  }
  Log-Step "[LAVCONTROL] \$Label failed after \$MaxAttempts attempt(s)"
  return \$false
}

function Install-BundledDriverPackages {
  param(
    [string]\$StagingDir,
    [string]\$MachineDir
  )
  \$arch = '${WindowsBinaryManager.windowsArch}'
  \$platform = if (\$arch -eq 'arm64') { 'arm64' } else { 'amd64' }
  \$dcoFlavor = Get-OvpnDcoOsFlavor
  \$installedAny = \$false

  \$driverJobs = @(
    @{ Label = 'ovpn-dco'; Relative = ("ovpn-dco/" + \$dcoFlavor + "/ovpn-dco.inf") },
    @{ Label = 'tap-windows6'; Relative = ("tap/" + \$platform + "/win10/OemVista.inf"); Fallback = ("tap/" + \$platform + "/OemVista.inf") }
  )
  if (\$arch -eq 'arm64') {
    # ARM: install TAP before DCO (TAP fallback is more reliable during bring-up).
    \$driverJobs = @(\$driverJobs[1], \$driverJobs[0])
  }

  foreach (\$root in @(
    (Join-Path \$StagingDir 'drivers'),
    (Join-Path \$MachineDir 'drivers')
  )) {
    if (-not (Test-Path \$root)) { continue }

    foreach (\$job in \$driverJobs) {
      \$inf = Join-Path \$root \$job.Relative
      if (-not (Test-Path -LiteralPath \$inf) -and \$job.Fallback) {
        \$inf = Join-Path \$root \$job.Fallback
      }
      if (Install-DriverPackageIfPresent -Label \$job.Label -InfPath \$inf) { \$installedAny = \$true }
    }
  }

  if (\$installedAny) {
    # Driver store / PnP stack needs a moment before tapctl can open DevRegKey (common on ARM).
    Start-Sleep -Seconds 3
  }
  return \$installedAny
}

function Install-OpenVpnDrivers {
  param(
    [string]\$StagingDir,
    [string]\$MachineDir
  )
  \$driversStaging = Join-Path \$StagingDir 'drivers'
  \$driversMachine = Join-Path \$MachineDir 'drivers'
  if (Test-Path \$driversStaging) {
    New-Item -ItemType Directory -Force -Path \$driversMachine | Out-Null
    Copy-Item -Path (Join-Path \$driversStaging '*') -Destination \$driversMachine -Recurse -Force -ErrorAction SilentlyContinue
    Log-Step "[LAVCONTROL] Driver packages copied to \$driversMachine"
  }

  Install-BundledDriverPackages -StagingDir \$StagingDir -MachineDir \$MachineDir | Out-Null

  \$nativeArch = \$env:PROCESSOR_ARCHITECTURE
  \$wowArch = \$env:PROCESSOR_ARCHITEW6432
  Log-Step "[LAVCONTROL] Host arch PROCESSOR_ARCHITECTURE=\$nativeArch PROCESSOR_ARCHITEW6432=\$wowArch bundleArch=${WindowsBinaryManager.windowsArch}"

  \$tapctlStaging = Join-Path \$StagingDir 'tapctl.exe'
  if (-not (Test-Path \$tapctlStaging)) {
    \$tapctlBin = Join-Path \$StagingDir 'tapctl.exe.bin'
    if (Test-Path \$tapctlBin) {
      Copy-Item -Path \$tapctlBin -Destination \$tapctlStaging -Force
    }
  }
  \$tapctl = Join-Path \$MachineDir 'tapctl.exe'
  if (Test-Path \$tapctlStaging) {
    Copy-Item -Path \$tapctlStaging -Destination \$tapctl -Force
    Log-Step "[LAVCONTROL] tapctl.exe deployed to \$tapctl"
  } elseif (-not (Test-Path \$tapctl)) {
    Log-Step "[LAVCONTROL] tapctl.exe missing; skipping adapter creation"
    return
  }

  \$dcoList = Invoke-NativeLogged -Label 'tapctl list (before)' -FilePath \$tapctl -ArgumentList @('list')
  Log-Step "[LAVCONTROL] tapctl list before install:`n\$dcoList"

  function New-DcoAdapter {
    Log-Step "[LAVCONTROL] Creating ovpn-dco adapter (Win-DCO)"
    Invoke-TapCtlCreateWithRetry -TapCtl \$tapctl -Label 'tapctl create ovpn-dco' -ArgumentList @(
      'create', '--hwid', 'ovpn-dco', '--name', 'OpenVPN Data Channel Offload'
    ) | Out-Null
  }

  function New-TapAdapter {
    Log-Step "[LAVCONTROL] Creating tap-windows6 adapter (fallback)"
    \$retryArgs = @{ TapCtl = \$tapctl; Label = 'tapctl create tap0901'; ArgumentList = @(
      'create', '--hwid', 'tap0901', '--name', 'OpenVPN TAP-Windows6'
    ) }
    if ('${WindowsBinaryManager.windowsArch}' -eq 'arm64') {
      \$retryArgs.MaxAttempts = 8
      \$retryArgs.DelaySeconds = 3
    }
    Invoke-TapCtlCreateWithRetry @retryArgs | Out-Null
  }

  if ('${WindowsBinaryManager.windowsArch}' -eq 'arm64') {
    if (\$dcoList -notmatch 'tap0901|TAP-Windows6') { New-TapAdapter }
    \$dcoList = Invoke-NativeLogged -Label 'tapctl list (mid)' -FilePath \$tapctl -ArgumentList @('list')
    if (\$dcoList -notmatch 'ovpn-dco|Data Channel Offload') { New-DcoAdapter }
  } else {
    if (\$dcoList -notmatch 'ovpn-dco|Data Channel Offload') { New-DcoAdapter }
    \$dcoList = Invoke-NativeLogged -Label 'tapctl list (mid)' -FilePath \$tapctl -ArgumentList @('list')
    if (\$dcoList -notmatch 'tap0901|TAP-Windows6') { New-TapAdapter }
  }

  Invoke-NativeLogged -Label 'tapctl list (after)' -FilePath \$tapctl -ArgumentList @('list') | Out-Null
}

Install-OpenVpnDrivers -StagingDir '$staging' -MachineDir '$machine'
''';
  }
}
