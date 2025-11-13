# Configuration CMake pour le plugin Windows OpenVPN Flutter

cmake_minimum_required(VERSION 3.14)
project(openvpn_flutter_windows)

# Configuration Flutter
set(FLUTTER_MANAGED_DIR "${CMAKE_CURRENT_SOURCE_DIR}/flutter/ephemeral")
include("${FLUTTER_MANAGED_DIR}/generated_plugin_registrant.cmake")

# Configuration Rust
set(RUST_DIR "${CMAKE_CURRENT_SOURCE_DIR}/rust")
set(RUST_TARGET_DIR "${RUST_DIR}/target")
set(RUST_LIB_NAME "openvpn_flutter_rust")

# Déterminer le mode (debug/release)
if(CMAKE_BUILD_TYPE STREQUAL "Debug")
    set(RUST_BUILD_TYPE "debug")
    set(RUST_LIB_PATH "${RUST_TARGET_DIR}/debug/${RUST_LIB_NAME}.dll")
else()
    set(RUST_BUILD_TYPE "release")
    set(RUST_LIB_PATH "${RUST_TARGET_DIR}/release/${RUST_LIB_NAME}.dll")
endif()

# Compiler la bibliothèque Rust
add_custom_command(
    OUTPUT ${RUST_LIB_PATH}
    COMMAND cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --target x86_64-pc-windows-msvc
    WORKING_DIRECTORY ${RUST_DIR}
    COMMENT "Building Rust library"
)

# Créer une cible pour la bibliothèque Rust
add_custom_target(rust_lib DEPENDS ${RUST_LIB_PATH})

# Sources C++
add_library(${PROJECT_NAME} SHARED
    "openvpn_flutter_plugin.cpp"
    "openvpn_flutter_plugin.h"
)

# Inclure les répertoires
target_include_directories(${PROJECT_NAME} PRIVATE
    "${FLUTTER_MANAGED_DIR}"
    "${RUST_TARGET_DIR}/x86_64-pc-windows-msvc/${RUST_BUILD_TYPE}"
)

# Lier avec Flutter
target_link_libraries(${PROJECT_NAME} PRIVATE flutter flutter_wrapper_plugin)

# Lier avec la bibliothèque Rust
target_link_libraries(${PROJECT_NAME} PRIVATE "${RUST_LIB_PATH}")

# Dépendance sur la compilation Rust
add_dependencies(${PROJECT_NAME} rust_lib)

# Copier la DLL Rust vers le répertoire de sortie
add_custom_command(TARGET ${PROJECT_NAME} POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
    "${RUST_LIB_PATH}"
    "$<TARGET_FILE_DIR:${PROJECT_NAME}>"
)

