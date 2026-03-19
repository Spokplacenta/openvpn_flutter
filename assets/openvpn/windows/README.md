# OpenVPN embedded runtime (Windows)

Place OpenVPN runtime binaries for Windows in this folder.

Recommended steps:
1. Collect signed files from an official OpenVPN installation (`bin` folder).
2. Add these files to this assets directory:
   - `openvpn.exe.bin` (or `openvpn.exe`)
   - `libcrypto-3-x64.dll` (or `libcrypto-3-x64.dll.bin`)
   - `libssl-3-x64.dll` (or `libssl-3-x64.dll.bin`)
   - `libpkcs11-helper-1.dll` (or `libpkcs11-helper-1.dll.bin`)
   - `wintun.dll.bin` (official AMD64 DLL from https://www.wintun.net/)
3. Update `WindowsBinaryManager.targetVersion` and `expectedHash` accordingly.
4. Validate license requirements before distribution.

The plugin deploys these DLLs next to `openvpn.exe` on first startup.
`WindowsWintunManager` also deploys `wintun.dll` in the same directory.

Important:
- TAP-Windows is no longer used, and `tap-windows-9.21.2.exe` must not be distributed.
- This repository may only contain this README on some branches.
  Add runtime binaries before distribution.
