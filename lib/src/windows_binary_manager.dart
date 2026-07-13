import 'dart:io';
import 'dart:typed_data';
import 'package:flutter/foundation.dart' show FlutterError, debugPrint;
import 'package:flutter/services.dart' show rootBundle;
import 'package:http/http.dart' as http;
import 'package:path_provider/path_provider.dart';
import 'package:path/path.dart' as path;
import 'package:crypto/crypto.dart' show sha256;

/// Manages download, verification and storage of openvpn.exe for Windows
class WindowsBinaryManager {
  /// Target OpenVPN version (to be updated according to official releases)
  /// See: https://github.com/OpenVPN/openvpn/releases
  static const String targetVersion = '2.6.16';

  /// Path to the embedded OpenVPN binary asset inside the Flutter bundle.
  /// Override the file located at assets/openvpn/windows/openvpn.exe.bin
  /// with a trusted executable before distributing the app.
  /// Note: For plugin assets, the path must include 'packages/plugin_name/'
  static const String embeddedAssetPath =
      'packages/openvpn_flutter/assets/openvpn/windows/openvpn.exe.bin';

  /// Indicates whether we should attempt to deploy the embedded asset before
  /// falling back to the legacy download mechanism.
  static const bool preferEmbeddedBinary = true;

  /// Base URL to download OpenVPN installer from GitHub Releases
  ///
  /// OpenVPN GitHub releases contain MSI installers, not standalone binaries.
  /// URL format: https://github.com/OpenVPN/openvpn/releases/download/v{VERSION}/openvpn-install-{VERSION}-I602-amd64.exe
  ///
  /// IMPORTANT: The MSI installer must be extracted to obtain openvpn.exe.
  /// Extraction options:
  /// 1. Use an MSI extraction tool (7-Zip, msiexec, etc.)
  /// 2. Install temporarily then copy openvpn.exe from Program Files
  /// 3. Use a pre-extracted binary from a trusted source
  ///
  /// For a production solution, consider:
  /// - Pre-extract openvpn.exe and distribute it with the application
  /// - Use a portable/standalone build if available
  /// - Compile from sources with openvpn-build
  static const String baseDownloadUrl =
      'https://github.com/OpenVPN/openvpn/releases/download/v$targetVersion/openvpn-install-$targetVersion-I001-amd64.exe';

  /// Expected SHA256 hash of the binary (to be filled once the source is defined)
  /// Allows verification of downloaded file integrity
  /// Checksums are available at: https://github.com/OpenVPN/openvpn/releases
  static const String? expectedHash =
      'F18248CAA052AE5F217BAF30BBFCCA44F8F55857610BA826AF8F1A8429F747DA';

  /// Indicates whether the downloaded file is an MSI installer (true) or a standalone binary (false)
  static const bool isInstaller = true;

  /// Binary file name
  static const String binaryName = 'openvpn.exe';

  /// Interactive Service binary (launches openvpn.exe as SYSTEM via named pipe).
  static const String interactiveServiceBinaryName = 'openvpnserv.exe';

  /// Message DLL used by the Interactive Service for event-log entries.
  static const String interactiveServiceMsgDll = 'openvpnservmsg.dll';

  /// SHA256 of the embedded `openvpnserv.exe` (OpenVPN 2.6.16).
  static const String? expectedInteractiveServiceHash =
      '19EC0DD386319FB543567A3AE426AA7980F4D1F844FC8EDB3CCB55E43B5DFA6E';

  /// Subfolder under the OpenVPN runtime directory for `.ovpn` configs.
  static const String configSubdir = 'config';

  /// Subfolder for OpenVPN log files (service mode).
  static const String logSubdir = 'log';

  /// Subfolder for OpenVPN status files.
  static const String statusSubdir = 'status';

  /// Version file name
  static const String versionFileName = 'openvpn_version.txt';

  /// Hash file name
  static const String hashFileName = 'openvpn_sha256.txt';

  /// OpenVPN runtime dependencies required by openvpn.exe on Windows.
  static const List<String> requiredRuntimeDlls = [
    'libcrypto-3-x64.dll',
    'libssl-3-x64.dll',
    'libpkcs11-helper-1.dll',
  ];

  /// Plugin asset base paths (package and direct paths).
  static const List<String> _assetBasePaths = [
    'packages/openvpn_flutter/assets/openvpn/windows',
    'assets/openvpn/windows',
  ];

  /// Environment variable allowing to override the binary path for debugging.
  ///
  /// If this variable is set (e.g. to a system-installed openvpn.exe),
  /// the manager will use that path directly instead of deploying the
  /// embedded asset or downloading anything.
  static const String overrideEnvVar = 'OPENVPN_WINDOWS_BINARY_OVERRIDE';

  /// Gets the storage directory for openvpn.exe
  static Future<Directory> _getStorageDirectory() async {
    final appSupportDir = await getApplicationSupportDirectory();
    final openvpnDir = Directory(path.join(appSupportDir.path, 'openvpn'));
    if (!await openvpnDir.exists()) {
      await openvpnDir.create(recursive: true);
    }
    return openvpnDir;
  }

  /// Directory where OpenVPN runtime binaries are stored.
  static Future<Directory> getRuntimeDirectory() => _getStorageDirectory();

  /// Directory approved by the Interactive Service for `.ovpn` configs.
  static Future<String> getConfigDirectory() async {
    final dir = await _getStorageDirectory();
    final configDir = Directory(path.join(dir.path, configSubdir));
    if (!await configDir.exists()) {
      await configDir.create(recursive: true);
    }
    return configDir.path;
  }

  /// Full path to the deployed Interactive Service binary.
  static Future<String> getInteractiveServicePath() async {
    final dir = await _getStorageDirectory();
    return path.join(dir.path, interactiveServiceBinaryName);
  }

  /// Gets the full path to openvpn.exe
  static Future<String> getBinaryPath() async {
    // Debug override: allow using a system-installed binary
    final overridePath = Platform.environment[overrideEnvVar];
    if (overridePath != null && overridePath.isNotEmpty) {
      return overridePath;
    }

    final dir = await _getStorageDirectory();
    return path.join(dir.path, binaryName);
  }

  /// Checks if openvpn.exe exists locally
  static Future<bool> binaryExists() async {
    final binaryPath = await getBinaryPath();
    final file = File(binaryPath);
    return await file.exists();
  }

  /// Gets the locally installed version
  static Future<String?> getInstalledVersion() async {
    final dir = await _getStorageDirectory();
    final versionFile = File(path.join(dir.path, versionFileName));
    if (await versionFile.exists()) {
      try {
        return await versionFile.readAsString();
      } catch (e) {
        return null;
      }
    }
    return null;
  }

  /// Saves the installed version
  static Future<void> _saveInstalledVersion(String version) async {
    final dir = await _getStorageDirectory();
    final versionFile = File(path.join(dir.path, versionFileName));
    await versionFile.writeAsString(version);
  }

  /// Calculates the SHA256 hash of a file
  static Future<String> calculateFileHash(File file) async {
    final bytes = await file.readAsBytes();
    final hash = sha256.convert(bytes);
    return hash.toString();
  }

  /// Checks if an update is needed
  static Future<bool> needsUpdate() async {
    final installedVersion = await getInstalledVersion();
    if (installedVersion == null) return true;
    return installedVersion != targetVersion;
  }

  /// Downloads the OpenVPN installer from GitHub Releases
  ///
  /// NOTE: If [isInstaller] is true, the downloaded file is an MSI that must be extracted.
  /// This method only downloads the installer. Extraction must be handled separately.
  ///
  /// [onProgress] optional callback to track progress (0.0 to 1.0)
  /// Returns the path to the downloaded file (MSI or exe depending on source)
  static Future<String> downloadBinary({
    Function(double progress)? onProgress,
  }) async {
    final dir = await _getStorageDirectory();
    final binaryPath = path.join(dir.path, binaryName);
    final file = File(binaryPath);

    try {
      // Download the file
      final response = await http.get(
        Uri.parse(baseDownloadUrl),
      );

      if (response.statusCode != 200) {
        throw Exception('Download failed: HTTP ${response.statusCode}');
      }

      // Write the file
      await file.writeAsBytes(response.bodyBytes);

      // Verify that the file exists and is not empty
      if (!await file.exists() || await file.length() == 0) {
        throw Exception('Downloaded file is empty or invalid');
      }

      // If it's an MSI installer, note that extraction will be necessary
      if (isInstaller && binaryPath.endsWith('.exe')) {
        // The downloaded file is the installer, not the final binary
        // Extraction will need to be handled by a separate method
      }

      await _verifyHash(file);

      // If it's an installer, extract openvpn.exe
      if (isInstaller) {
        final extractedPath = await _extractFromInstaller(file.path);
        // Replace the installer file with the extracted binary
        if (extractedPath != null && await File(extractedPath).exists()) {
          final sourceDirectoryPath = path.dirname(extractedPath);
          final targetDirectoryPath = path.dirname(binaryPath);
          await file.delete(); // Delete the installer
          await File(extractedPath).copy(binaryPath); // Copy the binary
          await _copyRuntimeDllsFromDirectory(
            sourceDirectoryPath: sourceDirectoryPath,
            targetDirectoryPath: targetDirectoryPath,
            failIfMissing: true,
          );
          await _deployInteractiveServiceBinaries(
            Directory(targetDirectoryPath),
          );
          await File(extractedPath).delete(); // Clean up temporary file
        } else {
          throw Exception('Failed to extract openvpn.exe from installer');
        }
      }

      // Save the version
      await _saveInstalledVersion(targetVersion);

      await _persistHashFile();

      return binaryPath;
    } catch (e) {
      // Clean up on error
      if (await file.exists()) {
        await file.delete();
      }
      rethrow;
    }
  }

  /// Checks and downloads openvpn.exe if necessary
  ///
  /// [forceUpdate] forces download even if a version already exists
  /// [onProgress] optional callback to track progress
  /// Returns the path to openvpn.exe (downloaded or existing)
  static Future<String> ensureBinary({
    bool forceUpdate = false,
    Function(double progress)? onProgress,
  }) async {
    final binaryPath = await getBinaryPath();

    // If an override is set, don't try to deploy or download anything.
    final overridePath = Platform.environment[overrideEnvVar];
    if (overridePath != null && overridePath.isNotEmpty) {
      final file = File(overridePath);
      if (await file.exists()) {
        debugPrint(
          '[WindowsBinaryManager] Using override OpenVPN binary from $overridePath',
        );
        return overridePath;
      } else {
        debugPrint(
          '[WindowsBinaryManager] Override path $overridePath does not exist; '
          'falling back to embedded/deployed binary.',
        );
      }
    }

    if (await binaryExists()) {
      if (!forceUpdate) {
        final needsUpdate = await WindowsBinaryManager.needsUpdate();
        if (!needsUpdate) {
          await _ensureRuntimeDependenciesForExistingBinary(binaryPath);
          return binaryPath;
        }
      }
      await _deleteExistingArtifacts();
    }

    Exception? embeddedError;
    if (preferEmbeddedBinary) {
      try {
        final deployed = await _deployEmbeddedBinary(onProgress: onProgress);
        if (deployed) {
          return binaryPath;
        }
        // If deployed is false, asset was not found
        embeddedError = Exception(
          'Le binaire OpenVPN embarqué n\'a pas pu être déployé. '
          'Vérifiez que le fichier assets/openvpn/windows/openvpn.exe.bin existe '
          'et est inclus dans le build de l\'application.'
        );
      } catch (e) {
        embeddedError =
            Exception('Échec du déploiement du binaire OpenVPN embarqué: $e');
      }
    }

    // If embedded binary failed and we prefer it, don't try to download
    if (preferEmbeddedBinary && embeddedError != null) {
      throw Exception(
        'Impossible d\'utiliser le binaire OpenVPN embarqué. '
        'Le téléchargement automatique est désactivé. '
        'Erreur: ${embeddedError.toString()}'
      );
    }

    try {
      return await downloadBinary(onProgress: onProgress);
    } catch (e) {
      if (embeddedError != null) {
        throw Exception(
          '${embeddedError.toString()}\nLe téléchargement de secours a également échoué: $e',
        );
      }
      rethrow;
    }
  }

  /// Checks binary execution permissions
  static Future<bool> checkPermissions() async {
    if (!Platform.isWindows) {
      return false;
    }

    final binaryPath = await getBinaryPath();
    final file = File(binaryPath);

    if (!await file.exists()) {
      return false;
    }

    // Check that the file is executable
    // On Windows, we just check existence and read permissions
    try {
      final stat = await file.stat();
      return stat.size > 0;
    } catch (e) {
      return false;
    }
  }

  /// Deletes the binary and associated files
  static Future<void> cleanup() async {
    final dir = await _getStorageDirectory();
    final binaryFile = File(path.join(dir.path, binaryName));
    final versionFile = File(path.join(dir.path, versionFileName));
    final hashFile = File(path.join(dir.path, hashFileName));

    if (await binaryFile.exists()) {
      await binaryFile.delete();
    }
    if (await versionFile.exists()) {
      await versionFile.delete();
    }
    if (await hashFile.exists()) {
      await hashFile.delete();
    }

    for (final dllName in requiredRuntimeDlls) {
      final dllFile = File(path.join(dir.path, dllName));
      if (await dllFile.exists()) {
        await dllFile.delete();
      }
    }
    for (final fileName in [
      interactiveServiceBinaryName,
      interactiveServiceMsgDll,
    ]) {
      final file = File(path.join(dir.path, fileName));
      if (await file.exists()) {
        await file.delete();
      }
    }
  }

  static Future<void> _deleteExistingArtifacts() async {
    final dir = await _getStorageDirectory();
    final filesToDelete = [
      path.join(dir.path, binaryName),
      path.join(dir.path, versionFileName),
      path.join(dir.path, hashFileName),
      ...requiredRuntimeDlls.map((dll) => path.join(dir.path, dll)),
    ];
    for (final filePath in filesToDelete) {
      final file = File(filePath);
      if (await file.exists()) {
        await file.delete();
      }
    }
  }

  static Future<void> _persistHashFile() async {
    if (expectedHash == null) return;
    final dir = await _getStorageDirectory();
    final hashFile = File(path.join(dir.path, hashFileName));
    await hashFile.writeAsString(expectedHash!);
  }

  static Future<void> _verifyHash(File file) async {
    if (expectedHash == null) return;
    final actualHash = await calculateFileHash(file);
    if (actualHash.toLowerCase() != expectedHash!.toLowerCase()) {
      await file.delete();
      throw Exception('OpenVPN binary hash mismatch. '
          'Expected: $expectedHash, Received: $actualHash');
    }
  }

  /// Gets the binary size in bytes
  static Future<int?> getBinarySize() async {
    final binaryPath = await getBinaryPath();
    final file = File(binaryPath);
    if (await file.exists()) {
      return await file.length();
    }
    return null;
  }

  /// Extracts openvpn.exe from the MSI installer
  ///
  /// Uses msiexec (native Windows) to extract files from the installer.
  /// Searches for openvpn.exe in the extracted files.
  ///
  /// Returns the path to the extracted openvpn.exe, or null if extraction fails.
  static Future<String?> _extractFromInstaller(String installerPath) async {
    if (!Platform.isWindows) {
      throw UnsupportedError('MSI extraction is only supported on Windows');
    }

    final dir = await _getStorageDirectory();
    final extractDir = Directory(path.join(dir.path, 'extract_temp'));

    // Create temporary extraction directory
    if (await extractDir.exists()) {
      await extractDir.delete(recursive: true);
    }
    await extractDir.create(recursive: true);

    try {
      // Use msiexec to extract files
      // /a = administration mode (extraction)
      // /qb = silent user interface
      // TARGETDIR = destination directory
      final result = await Process.run(
        'msiexec',
        [
          '/a',
          installerPath,
          '/qb',
          'TARGETDIR=${extractDir.path}',
        ],
        runInShell: true,
      );

      if (result.exitCode != 0) {
        throw Exception('MSI extraction failed: ${result.stderr}');
      }

      // Search for openvpn.exe in extracted files
      // It's usually located in: extractDir/PFILES/OpenVPN/bin/openvpn.exe
      // or: extractDir/Program Files/OpenVPN/bin/openvpn.exe
      final possiblePaths = [
        path.join(extractDir.path, 'PFILES', 'OpenVPN', 'bin', binaryName),
        path.join(
            extractDir.path, 'Program Files', 'OpenVPN', 'bin', binaryName),
        path.join(extractDir.path, 'OpenVPN', 'bin', binaryName),
      ];

      // Search recursively if standard paths don't work
      String? foundPath;
      for (final possiblePath in possiblePaths) {
        final file = File(possiblePath);
        if (await file.exists()) {
          foundPath = possiblePath;
          break;
        }
      }

      // If not found, search recursively
      foundPath ??= await _findBinaryRecursive(extractDir, binaryName);

      return foundPath;
    } catch (e) {
      // Clean up on error
      if (await extractDir.exists()) {
        await extractDir.delete(recursive: true);
      }
      rethrow;
    }
  }

  /// Recursively searches for a file in a directory
  static Future<String?> _findBinaryRecursive(
      Directory dir, String fileName) async {
    try {
      await for (final entity in dir.list(recursive: true)) {
        if (entity is File && path.basename(entity.path) == fileName) {
          return entity.path;
        }
      }
    } catch (e) {
      // Ignore permission errors
    }
    return null;
  }

  static Future<bool> _deployEmbeddedBinary({
    Function(double progress)? onProgress,
  }) async {
    final data = await _loadPluginAssetBytes('openvpn.exe.bin');

    if (data == null) {
      debugPrint(
          '❌ [OpenVPN] Aucun asset trouvé pour openvpn.exe.bin dans les chemins plugin.');
      return false;
    }

    // At this point, data is guaranteed to be non-null
    final loadedData = data;
    
    if (loadedData.lengthInBytes == 0) {
      throw Exception(
          'L’asset OpenVPN embarqué est vide. Remplace openvpn.exe.bin par un '
          'exécutable valide avant distribution.');
    }

    final binaryPath = await getBinaryPath();
    final tempPath = '$binaryPath.part';
    final tempFile = File(tempPath);

    await tempFile.writeAsBytes(
      loadedData.buffer.asUint8List(loadedData.offsetInBytes, loadedData.lengthInBytes),
    );

    await _verifyHash(tempFile);

    final targetFile = File(binaryPath);
    if (await targetFile.exists()) {
      await targetFile.delete();
    }
    await tempFile.rename(binaryPath);

    // Deploy runtime dependencies next to openvpn.exe.
    await _deployRuntimeDllsFromAssets(Directory(path.dirname(binaryPath)));
    await _deployInteractiveServiceBinaries(Directory(path.dirname(binaryPath)));

    await _saveInstalledVersion(targetVersion);
    await _persistHashFile();
    onProgress?.call(1.0);
    return true;
  }

  static Future<void> _ensureRuntimeDependenciesForExistingBinary(
      String binaryPath) async {
    final targetDir = Directory(path.dirname(binaryPath));
    final missing = await _missingRuntimeDlls(targetDir.path);
    if (missing.isNotEmpty) {
      debugPrint(
          '⚠️ [OpenVPN] DLL runtime manquantes détectées (${missing.join(", ")}), tentative de réparation via assets.');
      await _deployRuntimeDllsFromAssets(targetDir, failIfMissing: true);
    }

    // Toujours garantir la présence du service Interactive: les installations
    // antérieures à son introduction ont déjà openvpn.exe + DLL runtime, donc
    // le déploiement doit se faire même quand aucune DLL ne manque.
    await _deployInteractiveServiceBinaries(targetDir);
  }

  static Future<void> _deployInteractiveServiceBinaries(
    Directory targetDir, {
    bool failIfMissing = true,
  }) async {
    final servPath = path.join(
      targetDir.path,
      interactiveServiceBinaryName,
    );
    final servFile = File(servPath);
    if (!await servFile.exists()) {
      final data = await _loadPluginAssetBytes(
        '$interactiveServiceBinaryName.bin',
        allowBinSuffix: false,
      );
      if (data == null) {
        if (failIfMissing) {
          throw Exception(
            'Binaire Interactive Service manquant dans les assets: '
            '$interactiveServiceBinaryName.bin',
          );
        }
        return;
      }
      await _writeAssetToPath(data, servPath);
      if (expectedInteractiveServiceHash != null) {
        await _verifyHashWithExpected(
          File(servPath),
          expectedInteractiveServiceHash!,
        );
      }
      debugPrint('✅ [OpenVPN] Interactive Service déployé: $servPath');
    }

    final msgPath = path.join(targetDir.path, interactiveServiceMsgDll);
    if (!await File(msgPath).exists()) {
      final msgData = await _loadPluginAssetBytes(
        '$interactiveServiceMsgDll.bin',
        allowBinSuffix: false,
      );
      if (msgData != null) {
        await _writeAssetToPath(msgData, msgPath);
        debugPrint('✅ [OpenVPN] DLL service déployée: $msgPath');
      }
    }
  }

  static Future<void> _writeAssetToPath(ByteData data, String targetPath) async {
    final tempFile = File('$targetPath.part');
    await tempFile.writeAsBytes(
      data.buffer.asUint8List(data.offsetInBytes, data.lengthInBytes),
    );
    final targetFile = File(targetPath);
    if (await targetFile.exists()) {
      await targetFile.delete();
    }
    await tempFile.rename(targetPath);
  }

  static Future<void> _verifyHashWithExpected(
    File file,
    String expected,
  ) async {
    final actualHash = await calculateFileHash(file);
    if (actualHash.toLowerCase() != expected.toLowerCase()) {
      await file.delete();
      throw Exception(
        'Hash mismatch for ${file.path}. Expected: $expected, got: $actualHash',
      );
    }
  }

  static Future<void> _deployRuntimeDllsFromAssets(
    Directory targetDir, {
    bool failIfMissing = true,
  }) async {
    final missing = <String>[];

    for (final dllName in requiredRuntimeDlls) {
      final data = await _loadPluginAssetBytes(dllName, allowBinSuffix: true);
      if (data == null) {
        missing.add(dllName);
        continue;
      }

      final targetPath = path.join(targetDir.path, dllName);
      final tempFile = File('$targetPath.part');
      await tempFile.writeAsBytes(
        data.buffer.asUint8List(data.offsetInBytes, data.lengthInBytes),
      );

      final targetFile = File(targetPath);
      if (await targetFile.exists()) {
        await targetFile.delete();
      }
      await tempFile.rename(targetPath);
      debugPrint('✅ [OpenVPN] DLL déployée: $dllName');
    }

    if (missing.isNotEmpty && failIfMissing) {
      throw Exception(
        'DLL runtime OpenVPN manquantes dans les assets du plugin: ${missing.join(", ")}. '
        'Ajoute ces fichiers dans assets/openvpn/windows/.',
      );
    }
  }

  static Future<ByteData?> _loadPluginAssetBytes(
    String fileName, {
    bool allowBinSuffix = false,
  }) async {
    final candidates = <String>[];
    for (final basePath in _assetBasePaths) {
      candidates.add('$basePath/$fileName');
      if (allowBinSuffix && !fileName.endsWith('.bin')) {
        candidates.add('$basePath/$fileName.bin');
      }
    }

    for (final candidate in candidates) {
      try {
        final data = await rootBundle.load(candidate);
        debugPrint(
            '✅ [OpenVPN] Asset trouvé: $candidate (${data.lengthInBytes} bytes)');
        return data;
      } on FlutterError {
        // Try next candidate.
      } catch (e) {
        debugPrint('⚠️ [OpenVPN] Erreur chargement asset $candidate: $e');
      }
    }
    return null;
  }

  static Future<List<String>> _missingRuntimeDlls(String directoryPath) async {
    final missing = <String>[];
    for (final dllName in requiredRuntimeDlls) {
      final file = File(path.join(directoryPath, dllName));
      if (!await file.exists()) {
        missing.add(dllName);
      }
    }
    return missing;
  }

  static Future<void> _copyRuntimeDllsFromDirectory({
    required String sourceDirectoryPath,
    required String targetDirectoryPath,
    bool failIfMissing = true,
  }) async {
    final missing = <String>[];

    for (final dllName in requiredRuntimeDlls) {
      final sourcePath = await _findFileCaseInsensitive(
        sourceDirectoryPath,
        dllName,
      );
      if (sourcePath == null) {
        missing.add(dllName);
        continue;
      }

      final targetPath = path.join(targetDirectoryPath, dllName);
      await File(sourcePath).copy(targetPath);
      debugPrint('✅ [OpenVPN] DLL copiée depuis installeur: $dllName');
    }

    if (missing.isNotEmpty && failIfMissing) {
      throw Exception(
        'DLL runtime manquantes dans l’installeur OpenVPN: ${missing.join(", ")}',
      );
    }
  }

  static Future<String?> _findFileCaseInsensitive(
    String directoryPath,
    String fileName,
  ) async {
    final expected = fileName.toLowerCase();
    final direct = File(path.join(directoryPath, fileName));
    if (await direct.exists()) {
      return direct.path;
    }

    try {
      final dir = Directory(directoryPath);
      if (!await dir.exists()) {
        return null;
      }
      await for (final entity in dir.list()) {
        if (entity is File &&
            path.basename(entity.path).toLowerCase() == expected) {
          return entity.path;
        }
      }
    } catch (_) {
      // Ignore and report not found.
    }
    return null;
  }
}
