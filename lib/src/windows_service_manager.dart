import 'dart:io';

import 'package:flutter/foundation.dart' show debugPrint;
import 'package:flutter/services.dart' show rootBundle;
import 'package:path/path.dart' as path;
import 'package:path_provider/path_provider.dart';

import 'windows_binary_manager.dart';

/// Manages the OpenVPN Interactive Service on Windows.
///
/// The Interactive Service (`OpenVPNServiceInteractive`) runs as SYSTEM and
/// spawns `openvpn.exe` on behalf of unprivileged users, handling adapter
/// creation, route modification and DNS changes that require elevation.
///
/// This class handles detection, deployment and registration of the service.
class WindowsServiceManager {
  static const String serviceName = 'OpenVPNServiceInteractive';
  static const String serviceDisplayName =
      'OpenVPN Interactive Service';
  static const String serviceExeName = 'openvpnserv2.exe';

  static const List<String> _assetCandidates = [
    'packages/openvpn_flutter/assets/openvpn/windows/$serviceExeName.bin',
    'packages/openvpn_flutter/assets/openvpn/windows/$serviceExeName',
    'assets/openvpn/windows/$serviceExeName.bin',
    'assets/openvpn/windows/$serviceExeName',
  ];

  /// Returns `true` if an official OpenVPN installation is detected
  /// (registry key `HKLM\SOFTWARE\OpenVPN` with an `exe_path` value
  /// pointing to a file that exists).
  static Future<bool> isOfficialInstallDetected() async {
    if (!Platform.isWindows) return false;
    try {
      final result = await Process.run('reg', [
        'query',
        r'HKLM\SOFTWARE\OpenVPN',
        '/v',
        'exe_path',
      ]);
      if (result.exitCode != 0) return false;
      final output = result.stdout.toString();
      // Extract the path value after "REG_SZ"
      final match = RegExp(r'REG_SZ\s+(.+)').firstMatch(output);
      if (match == null) return false;
      final exePath = match.group(1)?.trim() ?? '';
      return exePath.isNotEmpty && await File(exePath).exists();
    } catch (_) {
      return false;
    }
  }

  /// Returns `true` if the service is registered in the Windows Service
  /// Control Manager.
  static Future<bool> isServiceInstalled() async {
    if (!Platform.isWindows) return false;
    final result = await Process.run('sc', ['query', serviceName]);
    return result.exitCode == 0;
  }

  /// Returns `true` if the service is currently running.
  static Future<bool> isServiceRunning() async {
    if (!Platform.isWindows) return false;
    final result = await Process.run('sc', ['query', serviceName]);
    if (result.exitCode != 0) return false;
    final output = result.stdout.toString();
    return output.contains('RUNNING');
  }

  /// Orchestrates service readiness: detect existing, deploy, register, start.
  ///
  /// This is safe to call multiple times; it is a no-op when the service is
  /// already installed and running.
  static Future<void> ensureServiceReady({
    Function(double progress)? onProgress,
  }) async {
    if (!Platform.isWindows) return;

    onProgress?.call(0.1);

    // 1. If the service is already running, nothing to do.
    if (await isServiceRunning()) {
      debugPrint('[OpenVPN] Interactive Service already running');
      onProgress?.call(1.0);
      return;
    }

    // 2. If an official OpenVPN installation exists with the service
    //    registered, just start it — don't overwrite registry settings.
    if (await isServiceInstalled()) {
      final isOfficial = await isOfficialInstallDetected();
      debugPrint(
        '[OpenVPN] Service installed (official=$isOfficial) but not running, starting...',
      );
      onProgress?.call(0.5);
      await _startService();
      onProgress?.call(1.0);
      return;
    }

    // 3. Service not installed — deploy our own and register (UAC).
    debugPrint('[OpenVPN] Service not installed, deploying...');
    onProgress?.call(0.2);
    final serviceExePath = await _deployServiceBinary();
    if (serviceExePath == null) {
      debugPrint(
        '[OpenVPN] openvpnserv2.exe asset not found — '
        'service installation skipped. '
        'Add the binary to assets/openvpn/windows/ before distribution.',
      );
      onProgress?.call(1.0);
      return;
    }
    onProgress?.call(0.4);
    await _registerAndStartService(serviceExePath);
    onProgress?.call(1.0);
  }

  // ---------------------------------------------------------------------------
  // Internal helpers
  // ---------------------------------------------------------------------------

  /// Deploy `openvpnserv2.exe` from Flutter assets next to `openvpn.exe`.
  static Future<String?> _deployServiceBinary() async {
    final binaryPath = await WindowsBinaryManager.getBinaryPath();
    final targetDir = path.dirname(binaryPath);
    final targetPath = path.join(targetDir, serviceExeName);

    if (await File(targetPath).exists()) {
      return targetPath;
    }

    // Try loading from plugin assets (same pattern as WindowsWintunManager).
    for (final candidate in _assetCandidates) {
      try {
        final data =
            await _loadAsset(candidate);
        if (data != null && data.isNotEmpty) {
          final tempFile = File('$targetPath.part');
          await tempFile.writeAsBytes(data);
          final target = File(targetPath);
          if (await target.exists()) {
            await target.delete();
          }
          await tempFile.rename(targetPath);
          debugPrint('[OpenVPN] Service binary deployed: $targetPath');
          return targetPath;
        }
      } catch (_) {
        // Try next candidate.
      }
    }
    return null;
  }

  static Future<List<int>?> _loadAsset(String key) async {
    try {
      final byteData = await rootBundle.load(key);
      return byteData.buffer.asUint8List(
        byteData.offsetInBytes,
        byteData.lengthInBytes,
      );
    } catch (_) {
      return null;
    }
  }

  /// Register the service and configure registry, then start it.
  /// This pops a UAC dialog.
  static Future<void> _registerAndStartService(String serviceExePath) async {
    final binaryPath = await WindowsBinaryManager.getBinaryPath();
    final openvpnDir = path.dirname(binaryPath);

    // Build a PowerShell script that:
    // 1. Creates registry keys
    // 2. Registers the service
    // 3. Starts the service
    final script = '''
# Create registry key for OpenVPN settings
New-Item -Path 'HKLM:\\SOFTWARE\\OpenVPN' -Force | Out-Null
Set-ItemProperty -Path 'HKLM:\\SOFTWARE\\OpenVPN' -Name 'exe_path' -Value '${binaryPath.replaceAll("'", "''")}'
Set-ItemProperty -Path 'HKLM:\\SOFTWARE\\OpenVPN' -Name 'config_dir' -Value '${openvpnDir.replaceAll("'", "''")}'
Set-ItemProperty -Path 'HKLM:\\SOFTWARE\\OpenVPN' -Name 'config_ext' -Value 'ovpn'
Set-ItemProperty -Path 'HKLM:\\SOFTWARE\\OpenVPN' -Name 'log_dir' -Value '${openvpnDir.replaceAll("'", "''")}'
Set-ItemProperty -Path 'HKLM:\\SOFTWARE\\OpenVPN' -Name 'log_append' -Value '1'
Set-ItemProperty -Path 'HKLM:\\SOFTWARE\\OpenVPN' -Name 'priority' -Value 'NORMAL_PRIORITY_CLASS'

# Stop existing service if present
sc.exe stop $serviceName 2>\$null

# Remove existing service if present
sc.exe delete $serviceName 2>\$null
Start-Sleep -Seconds 1

# Register the service
sc.exe create $serviceName binPath= '"${serviceExePath.replaceAll("'", "''")}"' start= auto type= own DisplayName= "$serviceDisplayName"

# Start the service
sc.exe start $serviceName
''';

    final tempDir = await getTemporaryDirectory();
    final scriptPath = path.join(tempDir.path, 'openvpn_service_setup.ps1');
    await File(scriptPath).writeAsString(script);

    debugPrint('[OpenVPN] Running service installation script with UAC...');

    final result = await Process.run('powershell', [
      '-Command',
      'Start-Process powershell -ArgumentList '
          "'-ExecutionPolicy Bypass -File \"$scriptPath\"' "
          '-Verb RunAs -Wait',
    ]);

    // Clean up script
    try {
      await File(scriptPath).delete();
    } catch (_) {}

    if (result.exitCode != 0) {
      debugPrint(
        '[OpenVPN] Service installation may have failed: '
        'exit=${result.exitCode} stderr=${result.stderr}',
      );
    }

    // Verify
    if (await isServiceRunning()) {
      debugPrint('[OpenVPN] Interactive Service installed and running');
    } else {
      debugPrint(
        '[OpenVPN] Service may not be running after installation. '
        'The user may have cancelled the UAC dialog.',
      );
    }
  }

  /// Try to start an already-registered service (may need elevation).
  static Future<void> _startService() async {
    var result = await Process.run('sc', ['start', serviceName]);
    if (result.exitCode != 0) {
      // Try with elevation
      debugPrint('[OpenVPN] sc start failed, trying with elevation...');
      result = await Process.run('powershell', [
        '-Command',
        "Start-Process sc.exe -ArgumentList 'start $serviceName' "
            '-Verb RunAs -Wait',
      ]);
    }
  }
}
