# OpenVPN embedded runtime (Windows)

Place OpenVPN runtime binaries for Windows in this folder.

Recommended steps:
1. Collect signed files from an official OpenVPN 2.6.x installation.
2. Add these files to this assets directory:

   From the `bin` folder:
   - `openvpn.exe.bin` (or `openvpn.exe`)
   - `libcrypto-3-x64.dll` (or `libcrypto-3-x64.dll.bin`)
   - `libssl-3-x64.dll` (or `libssl-3-x64.dll.bin`)
   - `libpkcs11-helper-1.dll` (or `libpkcs11-helper-1.dll.bin`)
   - `openvpnserv2.exe.bin` (or `openvpnserv2.exe`) — Interactive Service binary
   - `wintun.dll.bin` (official AMD64 DLL from https://www.wintun.net/)

3. Update `WindowsBinaryManager.targetVersion` and `expectedHash` accordingly.
4. Validate license requirements before distribution.

The plugin deploys these files next to `openvpn.exe` on first startup:
- `WindowsBinaryManager` deploys `openvpn.exe` and runtime DLLs.
- `WindowsWintunManager` deploys `wintun.dll`.
- `WindowsServiceManager` deploys `openvpnserv2.exe` and registers it
  as the OpenVPN Interactive Service (one-time UAC elevation).

The Interactive Service allows unprivileged users to create Wintun
adapters, modify routes and change DNS settings without running the
entire application as Administrator.

Important:
- TAP-Windows is no longer used; `tap-windows-9.21.2.exe` must not be distributed.
- This repository may only contain this README on some branches.
  Add runtime binaries before distribution.
