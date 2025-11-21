# CMake configuration for Windows OpenVPN Flutter plugin

cmake_minimum_required(VERSION 3.14)
project(openvpn_flutter_windows)

# Flutter configuration
# FLUTTER_MANAGED_DIR should be defined by the main project
# If not, we try to find it from the main project
if(NOT DEFINED FLUTTER_MANAGED_DIR)
    # Search from the main project directory
    # The plugin is in .plugin_symlinks/openvpn_flutter/windows
    # We go up to the ephemeral directory of the main project
    # Path: .../ephemeral/.plugin_symlinks/openvpn_flutter/windows
    get_filename_component(PLUGIN_DIR "${CMAKE_CURRENT_SOURCE_DIR}" DIRECTORY)
    # PLUGIN_DIR = .../openvpn_flutter
    get_filename_component(SYMLINKS_DIR "${PLUGIN_DIR}" DIRECTORY)
    # SYMLINKS_DIR = .../.plugin_symlinks
    get_filename_component(EPHEMERAL_DIR "${SYMLINKS_DIR}" DIRECTORY)
    # EPHEMERAL_DIR = .../ephemeral
    # FLUTTER_MANAGED_DIR should be .../ephemeral/flutter/ephemeral
    set(FLUTTER_MANAGED_DIR "${EPHEMERAL_DIR}/flutter/ephemeral")
endif()

# Note: generated_plugin_registrant.cmake is included by the main CMakeLists.txt
# of the Flutter project, not by the plugin. We don't need to include it here.

# Rust configuration
set(RUST_DIR "${CMAKE_CURRENT_SOURCE_DIR}/rust")
set(RUST_TARGET "x86_64-pc-windows-msvc")
set(RUST_TARGET_DIR "${RUST_DIR}/target")
set(RUST_LIB_NAME "openvpn_flutter_rust")
option(OPENVPN_FLUTTER_USE_PREBUILT_RUST "Use prebuilt Rust artifacts if available" ON)

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
set(RUST_IMPORT_LIB_NAME "${RUST_LIB_NAME}.lib")
set(RUST_DLL_PATH "${RUST_BUILD_OUTPUT_DIR}/${RUST_DLL_NAME}")
set(RUST_IMPORT_LIB_PATH "${RUST_BUILD_OUTPUT_DIR}/${RUST_IMPORT_LIB_NAME}")

set(RUST_PREBUILT_DIR "${RUST_DIR}/prebuilt/${RUST_TARGET}/${RUST_BUILD_TYPE}")
set(RUST_PREBUILT_DLL "${RUST_PREBUILT_DIR}/${RUST_DLL_NAME}")
set(RUST_PREBUILT_IMPORT_LIB "${RUST_PREBUILT_DIR}/${RUST_IMPORT_LIB_NAME}")

set(USE_PREBUILT_RUST FALSE)
if(OPENVPN_FLUTTER_USE_PREBUILT_RUST AND EXISTS ${RUST_PREBUILT_DLL} AND EXISTS ${RUST_PREBUILT_IMPORT_LIB})
    set(USE_PREBUILT_RUST TRUE)
    set(RUST_DLL_PATH "${RUST_PREBUILT_DLL}")
    set(RUST_IMPORT_LIB_PATH "${RUST_PREBUILT_IMPORT_LIB}")
else()
    if(OPENVPN_FLUTTER_USE_PREBUILT_RUST)
        message(WARNING "Prebuilt Rust artifacts not found (${RUST_PREBUILT_DLL}). Building locally via cargo.")
    endif()
endif()

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
add_library(${PROJECT_NAME} SHARED
    "openvpn_flutter_plugin.cpp"
    "openvpn_flutter_plugin.h"
)

# Include directories
target_include_directories(${PROJECT_NAME} PRIVATE "${FLUTTER_MANAGED_DIR}")

# Link with Flutter
target_link_libraries(${PROJECT_NAME} PRIVATE flutter flutter_wrapper_plugin)

# Link with Rust library
target_link_libraries(${PROJECT_NAME} PRIVATE "${RUST_IMPORT_LIB_PATH}")

# Dependency on Rust compilation
add_dependencies(${PROJECT_NAME} rust_lib)

# Copy Rust DLL to output directory
add_custom_command(TARGET ${PROJECT_NAME} POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
    "${RUST_DLL_PATH}"
    "$<TARGET_FILE_DIR:${PROJECT_NAME}>"
)

