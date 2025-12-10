import 'dart:io';

import 'package:flutter/foundation.dart' show debugPrint;
import 'package:flutter/services.dart' show rootBundle;
import 'package:path/path.dart' as path;

/// Gère la détection et l'installation du driver TAP/TUN sur Windows.
///
/// Le driver TAP/TUN est nécessaire pour qu'OpenVPN crée des interfaces réseau
/// virtuelles. Ce manager :
/// - détecte si un adaptateur TAP est déjà présent,
/// - sinon installe le driver à partir d'un exécutable TAP embarqué
///   dans les assets du plugin.
class WindowsTapManager {
  /// Nom du driver TAP dans le registre Windows
  static const String tapDriverName = 'TAP-Windows';
  
  /// Motif du nom d'adaptateur TAP
  static const String tapAdapterPattern = 'TAP-Windows Adapter';

  /// Chemin de l'asset embarqué contenant l'installeur TAP.
  ///
  /// Le fichier réel est dans:
  ///   assets/openvpn/windows/tap-windows-9.21.2.exe
  /// et doit être référencé côté Flutter via le préfixe 'packages/'.
  static const String embeddedTapInstallerAssetPath =
      'packages/openvpn_flutter/assets/openvpn/windows/tap-windows-9.21.2.exe';

  /// Checks if TAP/TUN driver is installed on the system
  /// 
  /// Returns true if at least one TAP adapter is found
  static Future<bool> isTapInstalled() async {
    if (!Platform.isWindows) {
      return false;
    }

    try {
      // Method 1: Check network adapters using PowerShell
      final result = await Process.run(
        'powershell',
        [
          '-Command',
          r'Get-NetAdapter | Where-Object { $_.Name -like "*TAP*" -or $_.InterfaceDescription -like "*TAP*" } | Measure-Object | Select-Object -ExpandProperty Count'
        ],
        runInShell: true,
      );

      if (result.exitCode == 0) {
        final countStr = result.stdout.toString().trim();
        final count = int.tryParse(countStr) ?? 0;
        debugPrint('[TAP Manager] Found $count TAP adapter(s)');
        return count > 0;
      }

      // Method 2: Check registry for TAP driver
      final regResult = await Process.run(
        'reg',
        [
          'query',
          'HKLM\\SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e972-e325-11ce-bfc1-08002be10318}',
          '/s',
          '/f',
          'TAP-Windows',
        ],
        runInShell: true,
      );

      if (regResult.exitCode == 0) {
        final output = regResult.stdout.toString();
        final hasTap = output.toLowerCase().contains('tap-windows');
        debugPrint('[TAP Manager] TAP driver found in registry: $hasTap');
        return hasTap;
      }

      return false;
    } catch (e) {
      debugPrint('[TAP Manager] Error checking TAP installation: $e');
      return false;
    }
  }

  /// Installe le driver TAP/TUN.
  ///
  /// Cette méthode utilise uniquement l'installeur TAP embarqué dans les assets.
  /// Nécessite les privilèges administrateur.
  ///
  /// [onProgress] callback optionnel (0.0 → 1.0).
  /// Retourne true si l'installation a réussi.
  static Future<bool> installTap({
    Function(double progress)? onProgress,
  }) async {
    if (!Platform.isWindows) {
      throw UnsupportedError('TAP installation is only supported on Windows');
    }

    // Check if already installed
    if (await isTapInstalled()) {
      debugPrint('[TAP Manager] TAP driver already installed');
      return true;
    }

    // Check admin privileges
    final hasAdmin = await hasAdminPrivileges();
    if (!hasAdmin) {
      throw Exception(
          'Administrator privileges are required to install TAP driver. '
          'Please run the application as administrator.');
    }

    try {
      debugPrint('[TAP Manager] Starting TAP driver installation from embedded asset...');

      // Charger l'installeur TAP embarqué depuis les assets
      final data = await rootBundle.load(embeddedTapInstallerAssetPath);
      if (data.lengthInBytes == 0) {
        throw Exception(
          'Embedded TAP installer asset is empty. '
          'Please ensure tap-windows-9.21.2.exe is correctly added to assets.',
        );
      }

      onProgress?.call(0.3);

      // Écrire l'installeur dans un fichier temporaire
      final tempDir = Directory.systemTemp;
      final installerPath = path.join(
        tempDir.path,
        'tap-windows-9.21.2-${DateTime.now().millisecondsSinceEpoch}.exe',
      );
      final installerFile = File(installerPath);

      await installerFile.writeAsBytes(
        data.buffer.asUint8List(
          data.offsetInBytes,
          data.lengthInBytes,
        ),
      );

      onProgress?.call(0.6);

      debugPrint('[TAP Manager] Running TAP installer silently: $installerPath');

      // Lancer l'installeur TAP en mode silencieux
      final installResult = await Process.run(
        installerPath,
        ['/S'], // Silent installation
        runInShell: true,
      );

      // Nettoyage de l'installeur
      try {
        if (await installerFile.exists()) {
          await installerFile.delete();
        }
      } catch (e) {
        debugPrint('[TAP Manager] Warning: Could not delete TAP installer: $e');
      }

      if (installResult.exitCode == 0) {
        // Laisser le temps au driver d'être enregistré
        await Future.delayed(const Duration(seconds: 2));

        // Vérifier l'installation
        final installed = await isTapInstalled();
        if (installed) {
          onProgress?.call(1.0);
          debugPrint('[TAP Manager] TAP driver installed successfully');
          return true;
        } else {
          debugPrint(
            '[TAP Manager] TAP installer completed but driver not detected. '
            'Please verify installation manually.',
          );
          return false;
        }
      } else {
        debugPrint(
          '[TAP Manager] TAP installer failed with exit code: '
          '${installResult.exitCode}',
        );
        debugPrint('[TAP Manager] TAP installer stderr: ${installResult.stderr}');
        return false;
      }
    } catch (e) {
      debugPrint(
        '[TAP Manager] Error installing TAP driver (asset or process issue): $e',
      );
      return false;
    }
  }


  /// Ensures TAP driver is installed
  /// 
  /// Checks if TAP is installed, and if not, attempts to install it.
  /// Returns true if TAP is available (either already installed or newly installed)
  static Future<bool> ensureTapInstalled({
    Function(double progress)? onProgress,
  }) async {
    if (await isTapInstalled()) {
      return true;
    }

    debugPrint('[TAP Manager] TAP driver not found, attempting installation...');
    return await installTap(onProgress: onProgress);
  }

  /// Checks if the current process has administrator privileges
  static Future<bool> hasAdminPrivileges() async {
    if (!Platform.isWindows) {
      return false;
    }

    try {
      // Try to create a file in a protected directory
      // This is a simple way to check admin privileges
      final testPath = r'C:\Windows\Temp\openvpn_admin_test.tmp';
      final testFile = File(testPath);
      
      try {
        await testFile.writeAsString('test');
        await testFile.delete();
        return true;
      } catch (e) {
        return false;
      }
    } catch (e) {
      return false;
    }
  }
}

