cmake_minimum_required(VERSION 3.20)
project(ArkPdbReader LANGUAGES C CXX)

# ---------------------------------------------------------------------------
# ArkPdbReader CMake integration
#
# This file does two things:
#   1. Provides a CMake target (ark-pdb-reader) that C++ consumers can
#      link against, pointing at the pre-built Rust shared library.
#   2. Optionally drives the Rust build via cargo if BUILD_RUST=ON.
#
# Quick start for C++ consumers:
#
#   find_package(ArkPdbReader REQUIRED PATHS /path/to/ArkPdbReader)
#   target_link_libraries(my_target PRIVATE ark-pdb-reader)
#
# Or, if integrating as a subdirectory:
#
#   add_subdirectory(ArkPdbReader)
#   target_link_libraries(my_target PRIVATE ark-pdb-reader)
# ---------------------------------------------------------------------------

# --- Options ----------------------------------------------------------------
option(ARK_PDB_READER_BUILD_RUST
    "Run 'cargo build --release' as part of the CMake build"
    ON)

option(ARK_PDB_READER_STATIC
    "Link the static archive instead of the shared library.  Not recommended on Windows (requires extra Rust runtime libs)."
    OFF)

# --- Paths ------------------------------------------------------------------
set(ARK_PDB_READER_SOURCE_DIR "${CMAKE_CURRENT_SOURCE_DIR}")
set(ARK_PDB_READER_INCLUDE_DIR "${ARK_PDB_READER_SOURCE_DIR}/include")

# When cross-compiling for Windows with a GNU/MinGW toolchain, cargo must be
# told the target triple explicitly and outputs to a target-specific subdir.
if(WIN32 AND CMAKE_CROSSCOMPILING AND CMAKE_CXX_COMPILER_ID STREQUAL "GNU")
    set(_ARK_PDB_DEFAULT_RUST_TARGET "x86_64-pc-windows-gnu")
else()
    set(_ARK_PDB_DEFAULT_RUST_TARGET "")
endif()

set(ARK_PDB_READER_RUST_TARGET "${_ARK_PDB_DEFAULT_RUST_TARGET}"
    CACHE STRING "Rust target triple passed to cargo (empty = host default)")

# Where Cargo puts release outputs.
if(ARK_PDB_READER_RUST_TARGET)
    set(ARK_PDB_READER_CARGO_TARGET_DIR
        "${ARK_PDB_READER_SOURCE_DIR}/target/${ARK_PDB_READER_RUST_TARGET}/release")
else()
    set(ARK_PDB_READER_CARGO_TARGET_DIR
        "${ARK_PDB_READER_SOURCE_DIR}/target/release")
endif()

# Resolve the library path(s) for the current platform.
if(ARK_PDB_READER_STATIC)
    if(WIN32)
        set(ARK_PDB_READER_LIB
            "${ARK_PDB_READER_CARGO_TARGET_DIR}/ark_pdb_reader.lib")
    else()
        set(ARK_PDB_READER_LIB
            "${ARK_PDB_READER_CARGO_TARGET_DIR}/libark_pdb_reader.a")
    endif()
else()
    if(WIN32)
        set(ARK_PDB_READER_LIB
            "${ARK_PDB_READER_CARGO_TARGET_DIR}/ark_pdb_reader.dll")
        # MinGW (GNU target) produces libark_pdb_reader.dll.a;
        # MSVC produces ark_pdb_reader.dll.lib.
        if(ARK_PDB_READER_RUST_TARGET MATCHES "windows-gnu" OR CMAKE_CXX_COMPILER_ID STREQUAL "GNU")
            set(ARK_PDB_READER_IMPLIB
                "${ARK_PDB_READER_CARGO_TARGET_DIR}/libark_pdb_reader.dll.a")
        else()
            set(ARK_PDB_READER_IMPLIB
                "${ARK_PDB_READER_CARGO_TARGET_DIR}/ark_pdb_reader.dll.lib")
        endif()
    else()
        set(ARK_PDB_READER_LIB
            "${ARK_PDB_READER_CARGO_TARGET_DIR}/libark_pdb_reader.so")
    endif()
endif()

# --- Optional Rust build step -----------------------------------------------
if(ARK_PDB_READER_BUILD_RUST)
    find_program(CARGO_EXECUTABLE cargo REQUIRED
        HINTS "$ENV{HOME}/.cargo/bin" "$ENV{USERPROFILE}/.cargo/bin"
        DOC "Path to the cargo Rust build tool")

    message(STATUS "ArkPdbReader: cargo found at ${CARGO_EXECUTABLE}")

    # The primary output that CMake tracks to know when a rebuild is needed.
    # On Windows this is the DLL (or .lib for static), on Linux the .so/.a.
    if(WIN32 AND NOT ARK_PDB_READER_STATIC)
        set(ARK_PDB_READER_CARGO_OUTPUT "${ARK_PDB_READER_LIB}")
    else()
        set(ARK_PDB_READER_CARGO_OUTPUT "${ARK_PDB_READER_LIB}")
    endif()

    if(ARK_PDB_READER_RUST_TARGET)
        set(_CARGO_TARGET_ARGS --target "${ARK_PDB_READER_RUST_TARGET}")
    else()
        set(_CARGO_TARGET_ARGS "")
    endif()

    add_custom_command(
        OUTPUT "${ARK_PDB_READER_CARGO_OUTPUT}"
        COMMAND "${CARGO_EXECUTABLE}" build --release ${_CARGO_TARGET_ARGS}
        WORKING_DIRECTORY "${ARK_PDB_READER_SOURCE_DIR}"
        COMMENT "Building ArkPdbReader Rust library (target: ${ARK_PDB_READER_RUST_TARGET})"
        VERBATIM
    )

    add_custom_target(ark-pdb-reader-cargo ALL
        DEPENDS "${ARK_PDB_READER_CARGO_OUTPUT}")
endif()

# --- Imported library target ------------------------------------------------
if(ARK_PDB_READER_STATIC)
    add_library(ark-pdb-reader STATIC IMPORTED GLOBAL)
    set_target_properties(ark-pdb-reader PROPERTIES
        IMPORTED_LOCATION "${ARK_PDB_READER_LIB}")
    # The Rust static archive needs these system libs on Linux.
    if(NOT WIN32)
        target_link_libraries(ark-pdb-reader INTERFACE dl pthread)
    endif()
else()
    add_library(ark-pdb-reader SHARED IMPORTED GLOBAL)
    set_target_properties(ark-pdb-reader PROPERTIES
        IMPORTED_LOCATION "${ARK_PDB_READER_LIB}"
        IMPORTED_NO_SONAME TRUE)
    # On Windows, the linker uses the import lib (.dll.lib), not the DLL
    # directly.  IMPORTED_IMPLIB tells CMake which file to pass to the linker.
    if(WIN32 AND DEFINED ARK_PDB_READER_IMPLIB)
        set_target_properties(ark-pdb-reader PROPERTIES
            IMPORTED_IMPLIB "${ARK_PDB_READER_IMPLIB}")
    endif()
endif()

target_include_directories(ark-pdb-reader INTERFACE
    "${ARK_PDB_READER_INCLUDE_DIR}")

if(ARK_PDB_READER_BUILD_RUST)
    add_dependencies(ark-pdb-reader ark-pdb-reader-cargo)
endif()

# --- Install ----------------------------------------------------------------
install(FILES
    "${ARK_PDB_READER_INCLUDE_DIR}/ark_pdb_reader.h"
    "${ARK_PDB_READER_INCLUDE_DIR}/ark_pdb_reader.hpp"
    DESTINATION include)

install(FILES "${ARK_PDB_READER_LIB}"
    DESTINATION lib)

if(WIN32 AND NOT ARK_PDB_READER_STATIC AND DEFINED ARK_PDB_READER_IMPLIB)
    install(FILES "${ARK_PDB_READER_IMPLIB}"
        DESTINATION lib)
endif()

message(STATUS "ArkPdbReader: library = ${ARK_PDB_READER_LIB}")
message(STATUS "ArkPdbReader: include = ${ARK_PDB_READER_INCLUDE_DIR}")
