import 'dart:io';
import 'package:http/http.dart' as http;
import 'package:path_provider/path_provider.dart';
import 'package:path/path.dart' as path;
import 'package:crypto/crypto.dart' show sha256;

/// Manages download, verification and storage of openvpn.exe for Windows
class WindowsBinaryManager {
  /// Target OpenVPN version (to be updated according to official releases)
  /// See: https://github.com/OpenVPN/openvpn/releases
  static const String targetVersion = '2.6.9';
  
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
      'https://github.com/OpenVPN/openvpn/releases/download/v$targetVersion/openvpn-install-$targetVersion-I602-amd64.exe';
  
  /// Expected SHA256 hash of the binary (to be filled once the source is defined)
  /// Allows verification of downloaded file integrity
  /// Checksums are available at: https://github.com/OpenVPN/openvpn/releases
  static const String? expectedHash = null;
  
  /// Indicates whether the downloaded file is an MSI installer (true) or a standalone binary (false)
  static const bool isInstaller = true;
  
  /// Binary file name
  static const String binaryName = 'openvpn.exe';
  
  /// Version file name
  static const String versionFileName = 'openvpn_version.txt';
  
  /// Hash file name
  static const String hashFileName = 'openvpn_sha256.txt';

  /// Gets the storage directory for openvpn.exe
  static Future<Directory> _getStorageDirectory() async {
    final appSupportDir = await getApplicationSupportDirectory();
    final openvpnDir = Directory(path.join(appSupportDir.path, 'openvpn'));
    if (!await openvpnDir.exists()) {
      await openvpnDir.create(recursive: true);
    }
    return openvpnDir;
  }

  /// Gets the full path to openvpn.exe
  static Future<String> getBinaryPath() async {
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
        throw Exception(
            'Download failed: HTTP ${response.statusCode}');
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

      // Verify hash if available
      if (expectedHash != null) {
        final actualHash = await calculateFileHash(file);
        if (actualHash.toLowerCase() != expectedHash!.toLowerCase()) {
          await file.delete();
          throw Exception(
              'Downloaded file hash does not match. '
              'Expected: $expectedHash, Received: $actualHash');
        }
      }

      // If it's an installer, extract openvpn.exe
      if (isInstaller) {
        final extractedPath = await _extractFromInstaller(file.path);
        // Replace the installer file with the extracted binary
        if (extractedPath != null && await File(extractedPath).exists()) {
          await file.delete(); // Delete the installer
          await File(extractedPath).copy(binaryPath); // Copy the binary
          await File(extractedPath).delete(); // Clean up temporary file
        } else {
          throw Exception('Failed to extract openvpn.exe from installer');
        }
      }

      // Save the version
      await _saveInstalledVersion(targetVersion);
      
      // Save the hash if available
      if (expectedHash != null) {
        final dir = await _getStorageDirectory();
        final hashFile = File(path.join(dir.path, hashFileName));
        await hashFile.writeAsString(expectedHash!);
      }

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
    // Check if binary exists and is up to date
    if (!forceUpdate && await binaryExists()) {
      final needsUpdate = await WindowsBinaryManager.needsUpdate();
      if (!needsUpdate) {
        return await getBinaryPath();
      }
      // Delete old version if an update is needed
      final oldBinary = File(await getBinaryPath());
      if (await oldBinary.exists()) {
        await oldBinary.delete();
      }
    }

    // Download the binary
    return await downloadBinary(onProgress: onProgress);
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
        throw Exception(
            'MSI extraction failed: ${result.stderr}');
      }

      // Search for openvpn.exe in extracted files
      // It's usually located in: extractDir/PFILES/OpenVPN/bin/openvpn.exe
      // or: extractDir/Program Files/OpenVPN/bin/openvpn.exe
      final possiblePaths = [
        path.join(extractDir.path, 'PFILES', 'OpenVPN', 'bin', binaryName),
        path.join(extractDir.path, 'Program Files', 'OpenVPN', 'bin', binaryName),
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
}

