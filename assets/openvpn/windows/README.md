# OpenVPN embedded runtime (Windows)

OpenVPN runtime binaries for Windows are embedded per **OS architecture** so the
same application works on both x64 and Windows on ARM (ARM64).

## Driver strategy (OpenVPN 2.7+)

OpenVPN 2.7 removed Wintun. LavControl uses:

1. **ovpn-dco** (Win-DCO) — default kernel driver
2. **tap-windows6** — fallback when DCO is unavailable

Adapters are created via `tapctl.exe` during elevated service setup
(`tapctl create --hwid ovpn-dco` and `tapctl create --hwid tap0901`).

Signed driver packages (ovpn-dco + tap-windows6) are bundled under
`drivers/` and installed with `pnputil` before adapter creation.
Run `tool/fetch_openvpn_binaries.ps1` to refresh binaries and drivers.

## Structure

```
assets/openvpn/windows/
  x64/     # amd64 build
    openvpn.exe.bin
    openvpnserv.exe.bin
    openvpnservmsg.dll.bin
    tapctl.exe.bin
    libcrypto-3-x64.dll
    libssl-3-x64.dll
    libpkcs11-helper-1.dll
  arm64/   # arm64 build (same layout)
```

`WindowsBinaryManager` deploys OpenVPN binaries. `WindowsDriverManager` deploys
`tapctl.exe` and creates DCO/TAP adapters under elevation.

## Current version

OpenVPN **2.7.5** (community Windows MSI I001, amd64 + arm64).

## How to refresh / bump the version

```
powershell -ExecutionPolicy Bypass -File tool/fetch_openvpn_binaries.ps1 -Version 2.7.5
```

Then update `WindowsBinaryManager.targetVersion` and SHA256 hashes as printed.

## Notes

- Verify MSI GPG signatures against official OpenVPN keys before distribution.
- Wintun is no longer bundled or used.
