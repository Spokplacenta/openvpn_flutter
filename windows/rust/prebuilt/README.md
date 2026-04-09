## Prebuilt Rust artifacts

This folder contains the Rust libraries shipped with `openvpn_flutter` so consumers can build on Windows without installing Rust/rustup for standard scenarios.

### Expected structure

```
windows/rust/prebuilt/
└── x86_64-pc-windows-msvc/
    ├── debug/
    │   ├── openvpn_flutter_rust.dll
    │   └── openvpn_flutter_rust.dll.lib
    └── release/
        ├── openvpn_flutter_rust.dll
        └── openvpn_flutter_rust.dll.lib
```

> Note: some toolchains generate `openvpn_flutter_rust.lib` instead of
> `openvpn_flutter_rust.dll.lib`. The plugin CMake accepts both names.

Only the `release` binaries are required for distributable builds (`flutter build windows`). `debug` binaries are useful for local testing without Rust.

### Generate/update binaries

1. Install a compatible MSVC toolchain (Visual Studio Build Tools), then run `rustup target add x86_64-pc-windows-msvc`.
2. From `windows/rust/`, run:

   ```powershell
   cargo build --target x86_64-pc-windows-msvc --release
   ```

   (Use `--profile dev` or omit `--release` to produce `debug` binaries.)
3. Copy generated files from `windows/rust/target/x86_64-pc-windows-msvc/<profile>/` into the `prebuilt/` structure above.
   - DLL: `openvpn_flutter_rust.dll`
   - Import library: `openvpn_flutter_rust.dll.lib` (or `openvpn_flutter_rust.lib`)
4. Commit these binaries so they are distributed with the library (or publish DLLs as release artifacts and place them here during packaging).

### CI validation

The GitHub Actions workflow `windows-rust-prebuilt.yml` rebuilds Windows artifacts and checks for drift against binaries versioned in `prebuilt/`.
If drift is detected, the workflow publishes a `prebuilt-drift.patch` artifact for review and repository update.

### MSVC linker error (LNK2019: `openvpn_is_service_available`, `openvpn_set_launch_mode`, ...)

The `.dll` / `.lib` files in this folder must export the same `#[no_mangle]` symbols as `windows/rust/src/lib.rs`.
If plugin C++ references a symbol missing from prebuilt artifacts (added after they were generated), **regenerate** binaries (see "Generate/update binaries") and commit them, **or** install `cargo` on the build machine: plugin `CMakeLists.txt` then prefers local compilation and ignores stale prebuilts.

### Disable prebuilt usage

When developing the plugin and you want to always rebuild Rust, disable the CMake option:

```powershell
cmake -DOPENVPN_FLUTTER_USE_PREBUILT_RUST=OFF ...
```

In this mode, CMake automatically invokes `cargo build` and uses artifacts from `windows/rust/target/`.

