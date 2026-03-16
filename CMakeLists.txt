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
    "Link the static archive (.a) instead of the shared library (.so)"
    OFF)

# --- Paths ------------------------------------------------------------------
set(ARK_PDB_READER_SOURCE_DIR "${CMAKE_CURRENT_SOURCE_DIR}")
set(ARK_PDB_READER_INCLUDE_DIR "${ARK_PDB_READER_SOURCE_DIR}/include")

# Where Cargo puts release outputs.
set(ARK_PDB_READER_CARGO_TARGET_DIR
    "${ARK_PDB_READER_SOURCE_DIR}/target/release")

if(ARK_PDB_READER_STATIC)
    set(ARK_PDB_READER_LIB
        "${ARK_PDB_READER_CARGO_TARGET_DIR}/libark_pdb_reader.a")
else()
    set(ARK_PDB_READER_LIB
        "${ARK_PDB_READER_CARGO_TARGET_DIR}/libark_pdb_reader.so")
endif()

# --- Optional Rust build step -----------------------------------------------
if(ARK_PDB_READER_BUILD_RUST)
    find_program(CARGO_EXECUTABLE cargo REQUIRED
        HINTS "$ENV{HOME}/.cargo/bin"
        DOC "Path to the cargo Rust build tool")

    message(STATUS "ArkPdbReader: cargo found at ${CARGO_EXECUTABLE}")

    add_custom_command(
        OUTPUT "${ARK_PDB_READER_LIB}"
        COMMAND "${CARGO_EXECUTABLE}" build --release
        WORKING_DIRECTORY "${ARK_PDB_READER_SOURCE_DIR}"
        COMMENT "Building ArkPdbReader Rust library"
        VERBATIM
    )

    add_custom_target(ark-pdb-reader-cargo ALL
        DEPENDS "${ARK_PDB_READER_LIB}")
endif()

# --- Imported library target ------------------------------------------------
if(ARK_PDB_READER_STATIC)
    add_library(ark-pdb-reader STATIC IMPORTED GLOBAL)
    set_target_properties(ark-pdb-reader PROPERTIES
        IMPORTED_LOCATION "${ARK_PDB_READER_LIB}")
    # The Rust static archive needs these system libs on Linux.
    target_link_libraries(ark-pdb-reader INTERFACE
        dl pthread)
else()
    add_library(ark-pdb-reader SHARED IMPORTED GLOBAL)
    set_target_properties(ark-pdb-reader PROPERTIES
        IMPORTED_LOCATION "${ARK_PDB_READER_LIB}"
        IMPORTED_NO_SONAME TRUE)
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

message(STATUS "ArkPdbReader: library = ${ARK_PDB_READER_LIB}")
message(STATUS "ArkPdbReader: include = ${ARK_PDB_READER_INCLUDE_DIR}")
