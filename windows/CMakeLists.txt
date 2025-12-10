# CMake configuration for Windows OpenVPN Flutter plugin

cmake_minimum_required(VERSION 3.14)
project(openvpn_flutter_windows)

# Flutter configuration
# Calculate EPHEMERAL_DIR from the plugin's location
# The plugin is in .plugin_symlinks/openvpn_flutter/windows
# Path: .../ephemeral/.plugin_symlinks/openvpn_flutter/windows
get_filename_component(PLUGIN_DIR "${CMAKE_CURRENT_SOURCE_DIR}" DIRECTORY)
# PLUGIN_DIR = .../openvpn_flutter
get_filename_component(SYMLINKS_DIR "${PLUGIN_DIR}" DIRECTORY)
# SYMLINKS_DIR = .../.plugin_symlinks
get_filename_component(EPHEMERAL_DIR "${SYMLINKS_DIR}" DIRECTORY ABSOLUTE)
# EPHEMERAL_DIR = .../ephemeral (e.g., windows/flutter/ephemeral) - now absolute
# FLUTTER_MANAGED_DIR is the same as EPHEMERAL_DIR for consistency
set(FLUTTER_MANAGED_DIR "${EPHEMERAL_DIR}")

# Note: generated_plugin_registrant.cmake is included by the main CMakeLists.txt
# of the Flutter project, not by the plugin. We don't need to include it here.

# Rust configuration
set(RUST_DIR "${CMAKE_CURRENT_SOURCE_DIR}/rust")
set(RUST_TARGET "x86_64-pc-windows-msvc")
set(RUST_TARGET_DIR "${RUST_DIR}/target")
set(RUST_LIB_NAME "openvpn_flutter_rust")

# IMPORTANT: pour le debug, on force la compilation locale du Rust
# plutôt que d'utiliser les DLL précompilées.
option(OPENVPN_FLUTTER_USE_PREBUILT_RUST "Use prebuilt Rust artifacts if available" OFF)

# Determine build mode (debug/release)
if(CMAKE_BUILD_TYPE STREQUAL "Debug")
    set(RUST_BUILD_TYPE "debug")
    set(CARGO_PROFILE_ARG "")
else()
    set(RUST_BUILD_TYPE "release")
    set(CARGO_PROFILE_ARG "--release")
endif()

set(RUST_BUILD_OUTPUT_DIR "${RUST_TARGET_DIR}/${RUST_TARGET}/${RUST_BUILD_TYPE}")
set(RUST_DLL_NAME "${RUST_LIB_NAME}.dll")
# Rust generates .dll.lib as the import library name, not just .lib
set(RUST_IMPORT_LIB_NAME "${RUST_LIB_NAME}.dll.lib")
set(RUST_DLL_PATH "${RUST_BUILD_OUTPUT_DIR}/${RUST_DLL_NAME}")
set(RUST_IMPORT_LIB_PATH "${RUST_BUILD_OUTPUT_DIR}/${RUST_IMPORT_LIB_NAME}")

set(RUST_PREBUILT_DIR "${RUST_DIR}/prebuilt/${RUST_TARGET}/${RUST_BUILD_TYPE}")
set(RUST_PREBUILT_DLL "${RUST_PREBUILT_DIR}/${RUST_DLL_NAME}")
set(RUST_PREBUILT_IMPORT_LIB "${RUST_PREBUILT_DIR}/${RUST_IMPORT_LIB_NAME}")

set(USE_PREBUILT_RUST FALSE)
# NOTE: même si des artefacts précompilés existent, on ne les utilise plus
# pour s'assurer que le code Rust modifié est bien pris en compte.

if(NOT USE_PREBUILT_RUST)
    # Build the Rust library
    add_custom_command(
        OUTPUT ${RUST_DLL_PATH}
        BYPRODUCTS ${RUST_IMPORT_LIB_PATH}
        COMMAND cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --target ${RUST_TARGET} ${CARGO_PROFILE_ARG}
        WORKING_DIRECTORY ${RUST_DIR}
        COMMENT "Building Rust library (${RUST_BUILD_TYPE})"
    )

    # Create a target for the Rust library
    add_custom_target(rust_lib DEPENDS ${RUST_DLL_PATH})
else()
    add_custom_target(rust_lib)
endif()

# C++ sources
# Use a consistent target name that Flutter expects
set(PLUGIN_TARGET_NAME "openvpn_flutter_plugin")
add_library(${PLUGIN_TARGET_NAME} SHARED
    "openvpn_flutter_plugin.cpp"
    "openvpn_flutter_plugin.h"
)

# Include directories
# - PUBLIC: expose the plugin's root dir so apps can include
#   "openvpn_flutter/openvpn_flutter_plugin.h"
# - PRIVATE: Flutter generated headers and wrapper.
target_include_directories(${PLUGIN_TARGET_NAME}
    PUBLIC
        "${CMAKE_CURRENT_SOURCE_DIR}"
    PRIVATE 
        "${EPHEMERAL_DIR}"
        "${EPHEMERAL_DIR}/cpp_client_wrapper/include"
)

# Link with Flutter
target_link_libraries(${PLUGIN_TARGET_NAME} PRIVATE flutter flutter_wrapper_plugin)

# Link with Rust library
target_link_libraries(${PLUGIN_TARGET_NAME} PRIVATE "${RUST_IMPORT_LIB_PATH}")

# Dependency on Rust compilation
add_dependencies(${PLUGIN_TARGET_NAME} rust_lib)

# Copy Rust DLL to output directory
# Register the DLL to be installed with the plugin
# Flutter will handle the installation during the install step
set(openvpn_flutter_bundled_libraries "${RUST_DLL_PATH}" PARENT_SCOPE)

