# Configuration CMake pour le plugin Windows OpenVPN Flutter

cmake_minimum_required(VERSION 3.14)
project(openvpn_flutter_windows)

# Configuration Flutter
set(FLUTTER_MANAGED_DIR "${CMAKE_CURRENT_SOURCE_DIR}/flutter/ephemeral")
include("${FLUTTER_MANAGED_DIR}/generated_plugin_registrant.cmake")

# Configuration Rust
set(RUST_DIR "${CMAKE_CURRENT_SOURCE_DIR}/rust")
set(RUST_TARGET "x86_64-pc-windows-msvc")
set(RUST_TARGET_DIR "${RUST_DIR}/target")
set(RUST_LIB_NAME "openvpn_flutter_rust")
option(OPENVPN_FLUTTER_USE_PREBUILT_RUST "Utiliser les artefacts Rust précompilés si disponibles" ON)

# Déterminer le mode (debug/release)
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
        message(WARNING "Artefacts Rust précompilés introuvables (${RUST_PREBUILT_DLL}). Compilation locale via cargo.")
    endif()
endif()

if(NOT USE_PREBUILT_RUST)
    # Compiler la bibliothèque Rust
    add_custom_command(
        OUTPUT ${RUST_DLL_PATH}
        BYPRODUCTS ${RUST_IMPORT_LIB_PATH}
        COMMAND cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --target ${RUST_TARGET} ${CARGO_PROFILE_ARG}
        WORKING_DIRECTORY ${RUST_DIR}
        COMMENT "Building Rust library (${RUST_BUILD_TYPE})"
    )

    # Créer une cible pour la bibliothèque Rust
    add_custom_target(rust_lib DEPENDS ${RUST_DLL_PATH})
else()
    add_custom_target(rust_lib)
endif()

# Sources C++
add_library(${PROJECT_NAME} SHARED
    "openvpn_flutter_plugin.cpp"
    "openvpn_flutter_plugin.h"
)

# Inclure les répertoires
target_include_directories(${PROJECT_NAME} PRIVATE "${FLUTTER_MANAGED_DIR}")

# Lier avec Flutter
target_link_libraries(${PROJECT_NAME} PRIVATE flutter flutter_wrapper_plugin)

# Lier avec la bibliothèque Rust
target_link_libraries(${PROJECT_NAME} PRIVATE "${RUST_IMPORT_LIB_PATH}")

# Dépendance sur la compilation Rust
add_dependencies(${PROJECT_NAME} rust_lib)

# Copier la DLL Rust vers le répertoire de sortie
add_custom_command(TARGET ${PROJECT_NAME} POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
    "${RUST_DLL_PATH}"
    "$<TARGET_FILE_DIR:${PROJECT_NAME}>"
)

