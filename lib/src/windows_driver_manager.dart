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

  /// Copies bundled tapctl.exe to the staging directory.
  static Future<bool> ensureDriversDeployed({
    void Function(double progress)? onProgress,
  }) async {
    if (!Platform.isWindows) return true;
    onProgress?.call(0.2);

    final data = await _loadTapCtlAsset();
    if (data == null || data.lengthInBytes == 0) {
      debugPrint('⚠️ [OpenVPN] tapctl.exe asset missing in bundle');
      onProgress?.call(1.0);
      return false;
    }

    final dest = await getStagingTapCtlPath();
    await File(dest).writeAsBytes(data.buffer.asUint8List(), flush: true);
    debugPrint('✅ [OpenVPN] tapctl deployed: $dest');
    onProgress?.call(1.0);
    return true;
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
  static String installDriversScriptBody({
    required String stagingDir,
    required String machineDir,
  }) {
    final staging = stagingDir.replaceAll("'", "''");
    final machine = machineDir.replaceAll("'", "''");
    return '''
function Install-OpenVpnDrivers {
  param(
    [string]\$StagingDir,
    [string]\$MachineDir
  )
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

  \$dcoList = & \$tapctl list 2>&1 | Out-String
  Log-Step "[LAVCONTROL] tapctl list before install:`n\$dcoList"
  if (\$dcoList -notmatch 'ovpn-dco|Data Channel Offload') {
    Log-Step "[LAVCONTROL] Creating ovpn-dco adapter (Win-DCO)"
    & \$tapctl create --hwid ovpn-dco 2>&1 | Out-String | ForEach-Object { Log-Step \$_ }
  } else {
    Log-Step "[LAVCONTROL] ovpn-dco adapter already present"
  }

  \$tapList = & \$tapctl list 2>&1 | Out-String
  if (\$tapList -notmatch 'tap0901|TAP-Windows6') {
    Log-Step "[LAVCONTROL] Creating tap-windows6 adapter (fallback)"
    & \$tapctl create --hwid tap0901 2>&1 | Out-String | ForEach-Object { Log-Step \$_ }
  } else {
    Log-Step "[LAVCONTROL] TAP adapter already present"
  }
}

Install-OpenVpnDrivers -StagingDir '$staging' -MachineDir '$machine'
''';
  }
}
