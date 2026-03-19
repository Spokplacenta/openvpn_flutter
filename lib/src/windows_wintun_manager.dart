import 'dart:io';

import 'package:flutter/foundation.dart' show FlutterError, debugPrint;
import 'package:flutter/services.dart' show ByteData, rootBundle;
import 'package:path/path.dart' as path;

import 'windows_binary_manager.dart';

/// Deploys the Wintun runtime DLL required by OpenVPN on Windows.
///
/// Unlike TAP, Wintun does not require a driver installer. The signed
/// `wintun.dll` is copied next to `openvpn.exe` and loaded by OpenVPN.
class WindowsWintunManager {
  static const String wintunFileName = 'wintun.dll';

  static const List<String> _assetCandidates = [
    'packages/openvpn_flutter/assets/openvpn/windows/wintun.dll.bin',
    'packages/openvpn_flutter/assets/openvpn/windows/wintun.dll',
    'assets/openvpn/windows/wintun.dll.bin',
    'assets/openvpn/windows/wintun.dll',
  ];

  /// Returns true when `wintun.dll` is already present next to `openvpn.exe`.
  static Future<bool> isWintunDeployed() async {
    if (!Platform.isWindows) {
      return false;
    }

    final wintunPath = await _getWintunPath();
    return File(wintunPath).exists();
  }

  /// Ensures `wintun.dll` is present next to `openvpn.exe`.
  ///
  /// Returns `true` if already present or successfully deployed.
  static Future<bool> ensureWintunDeployed({
    Function(double progress)? onProgress,
  }) async {
    if (!Platform.isWindows) {
      return true;
    }

    if (await isWintunDeployed()) {
      return true;
    }

    final data = await _loadWintunAsset();
    if (data == null || data.lengthInBytes == 0) {
      throw Exception(
        'Wintun asset missing or empty. Add assets/openvpn/windows/wintun.dll.bin '
        'before distributing the application.',
      );
    }

    onProgress?.call(0.5);

    final wintunPath = await _getWintunPath();
    final targetFile = File(wintunPath);
    final tempFile = File('$wintunPath.part');
    await tempFile.writeAsBytes(
      data.buffer.asUint8List(data.offsetInBytes, data.lengthInBytes),
    );

    if (await targetFile.exists()) {
      await targetFile.delete();
    }
    await tempFile.rename(wintunPath);
    debugPrint('✅ [OpenVPN] Wintun deployed: $wintunPath');

    onProgress?.call(1.0);
    return true;
  }

  static Future<String> _getWintunPath() async {
    final binaryPath = await WindowsBinaryManager.getBinaryPath();
    final openvpnDir = path.dirname(binaryPath);
    final dir = Directory(openvpnDir);
    if (!await dir.exists()) {
      await dir.create(recursive: true);
    }
    return path.join(openvpnDir, wintunFileName);
  }

  static Future<ByteData?> _loadWintunAsset() async {
    for (final candidate in _assetCandidates) {
      try {
        final data = await rootBundle.load(candidate);
        debugPrint(
          '✅ [OpenVPN] Wintun asset found: $candidate (${data.lengthInBytes} bytes)',
        );
        return data;
      } on FlutterError {
        // Try next candidate.
      } catch (e) {
        debugPrint('⚠️ [OpenVPN] Failed loading asset $candidate: $e');
      }
    }
    return null;
  }
}
