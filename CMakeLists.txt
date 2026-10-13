# Copyright 2017 Igalia S.L. All Rights Reserved.
#
# Distributed under MIT license.
# See file LICENSE for detail or copy at https://opensource.org/licenses/MIT

# The library is implemented in Rust (see src/); the thin C++ layer in cpp/
# provides the unchanged C++ API and ABI of libwoff2common, libwoff2dec and
# libwoff2enc. Building requires cargo/rustc in addition to a C++ compiler.
cmake_minimum_required(VERSION 3.10)

project(woff2)

include(GNUInstallDirs)

# Build options
option(BUILD_SHARED_LIBS "Build shared libraries" ON)
option(CANONICAL_PREFIXES "Canonical prefixes" OFF)
option(NOISY_LOGGING "Noisy logging" ON)

# Version information
set(WOFF2_VERSION 1.0.2)

# When building shared libraries it is important to set the correct rpath
# See https://cmake.org/Wiki/CMake_RPATH_handling#Always_full_RPATH
set(CMAKE_SKIP_BUILD_RPATH FALSE)
set(CMAKE_BUILD_WITH_INSTALL_RPATH FALSE)
set(CMAKE_INSTALL_RPATH_USE_LINK_PATH TRUE)
list(FIND CMAKE_PLATFORM_IMPLICIT_LINK_DIRECTORIES "${CMAKE_INSTALL_LIBDIR}" isSystemDir)
if ("${isSystemDir}" STREQUAL "-1")
  set(CMAKE_INSTALL_RPATH "${CMAKE_INSTALL_LIBDIR}")
endif()

# Find Brotli dependencies
set(CMAKE_MODULE_PATH "${CMAKE_CURRENT_SOURCE_DIR}/cmake")
find_package(BrotliDec)
if (NOT BROTLIDEC_FOUND)
    message(FATAL_ERROR "librotlidec is needed to build woff2.")
endif ()
find_package(BrotliEnc)
if (NOT BROTLIENC_FOUND)
    message(FATAL_ERROR "librotlienc is needed to build woff2.")
endif ()

# Find the Rust toolchain
find_program(CARGO cargo HINTS "$ENV{CARGO_HOME}/bin" "$ENV{HOME}/.cargo/bin")
if (NOT CARGO)
    message(FATAL_ERROR "cargo (https://rustup.rs) is needed to build woff2.")
endif ()
set(WOFF2_CARGO_PROFILE "release" CACHE STRING
    "Cargo profile used for the Rust implementation (release or dev)")
set(WOFF2_CARGO_TARGET "" CACHE STRING
    "Optional Rust target triple, for cross compilation")

# Set compiler flags
if (NOT CANONICAL_PREFIXES)
    add_definitions(-no-canonical-prefixes)
  endif ()
if (NOISY_LOGGING)
    add_definitions(-DFONT_COMPRESSION_BIN)
endif ()
add_definitions(-D__STDC_FORMAT_MACROS)
set(COMMON_FLAGS -fno-omit-frame-pointer)

if(${CMAKE_SYSTEM_NAME} MATCHES "Darwin")
    add_definitions(-DOS_MACOSX)
else ()
    set(COMMON_FLAGS "${COMMON_FLAG} -fno-omit-frame-pointer")
endif()

set(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} ${COMMON_FLAG}")

set(CMAKE_CXX_FLAGS "${CMAKE_CXX_FLAGS} ${COMMON_FLAG}")
set(CMAKE_CXX_STANDARD 11)

# Set search path for our private/public headers as well as Brotli headers
include_directories("cpp" "include"
                    "${BROTLIDEC_INCLUDE_DIRS}" "${BROTLIENC_INCLUDE_DIRS}")

# ---------------------------------------------------------------------------
# Rust implementation
# ---------------------------------------------------------------------------

set(CARGO_MANIFEST "${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml")
set(CARGO_TARGET_DIR "${CMAKE_CURRENT_BINARY_DIR}/cargo")
if (WOFF2_CARGO_PROFILE STREQUAL "dev")
  set(CARGO_PROFILE_DIR "debug")
else ()
  set(CARGO_PROFILE_DIR "${WOFF2_CARGO_PROFILE}")
endif ()
set(CARGO_FLAGS --manifest-path "${CARGO_MANIFEST}" --profile "${WOFF2_CARGO_PROFILE}")
if (WOFF2_CARGO_TARGET)
  list(APPEND CARGO_FLAGS --target "${WOFF2_CARGO_TARGET}")
  set(CARGO_OUT_DIR "${WOFF2_CARGO_TARGET}/${CARGO_PROFILE_DIR}")
else ()
  set(CARGO_OUT_DIR "${CARGO_PROFILE_DIR}")
endif ()

set(WOFF2_RS_FEATURES "capi")
set(WOFF2_BIN_FEATURES "system-brotli")
if (NOISY_LOGGING)
  set(WOFF2_RS_FEATURES "${WOFF2_RS_FEATURES},noisy-logging")
  set(WOFF2_BIN_FEATURES "${WOFF2_BIN_FEATURES},noisy-logging")
endif ()

if (CMAKE_VERSION VERSION_LESS 3.12)
  file(GLOB_RECURSE WOFF2_RS_SOURCES "${CMAKE_CURRENT_SOURCE_DIR}/src/*.rs")
else ()
  file(GLOB_RECURSE WOFF2_RS_SOURCES CONFIGURE_DEPENDS
       "${CMAKE_CURRENT_SOURCE_DIR}/src/*.rs")
endif ()
list(APPEND WOFF2_RS_SOURCES "${CARGO_MANIFEST}" "${CMAKE_CURRENT_SOURCE_DIR}/build.rs")

# The Rust staticlib. It does not link brotli itself: libwoff2dec and
# libwoff2enc pass in the brotli entry points.
set(WOFF2_RS_LIB
    "${CARGO_TARGET_DIR}/lib/${CARGO_OUT_DIR}/${CMAKE_STATIC_LIBRARY_PREFIX}woff2${CMAKE_STATIC_LIBRARY_SUFFIX}")
add_custom_command(
  OUTPUT "${WOFF2_RS_LIB}"
  COMMAND "${CMAKE_COMMAND}" -E env "CARGO_TARGET_DIR=${CARGO_TARGET_DIR}/lib"
          "${CARGO}" rustc ${CARGO_FLAGS} --lib --crate-type staticlib
          --no-default-features --features "${WOFF2_RS_FEATURES}"
          -- -C symbol-mangling-version=v0
  DEPENDS ${WOFF2_RS_SOURCES}
  COMMENT "Building the Rust implementation (cargo)"
  USES_TERMINAL
  VERBATIM)
add_custom_target(woff2_rs_build DEPENDS "${WOFF2_RS_LIB}")

# System libraries needed by the Rust standard library.
if (APPLE OR WIN32)
  set(WOFF2_RS_NATIVE_LIBS "")
else ()
  set(THREADS_PREFER_PTHREAD_FLAG ON)
  find_package(Threads REQUIRED)
  set(WOFF2_RS_NATIVE_LIBS Threads::Threads ${CMAKE_DL_LIBS} m)
  if (NOT ${CMAKE_SYSTEM_NAME} MATCHES "BSD")
    list(APPEND WOFF2_RS_NATIVE_LIBS rt)
  endif ()
endif ()
set(WOFF2_RS_PC_LIBS_PRIVATE "")
foreach(lib ${WOFF2_RS_NATIVE_LIBS})
  if (lib MATCHES "^-")
    set(WOFF2_RS_PC_LIBS_PRIVATE "${WOFF2_RS_PC_LIBS_PRIVATE} ${lib}")
  elseif (lib STREQUAL "Threads::Threads")
    set(WOFF2_RS_PC_LIBS_PRIVATE "${WOFF2_RS_PC_LIBS_PRIVATE} -lpthread")
  else ()
    set(WOFF2_RS_PC_LIBS_PRIVATE "${WOFF2_RS_PC_LIBS_PRIVATE} -l${lib}")
  endif ()
endforeach()

# Only export the woff2 C++ API (and the private woff2_rs_* entry points)
# from the shared libraries; hide everything coming from Rust.
# libwoff2common additionally exports the woff2_rs_* entry points used by
# libwoff2dec and libwoff2enc.
function(woff2_export_api target list)
  if (NOT BUILD_SHARED_LIBS)
    return()
  endif ()
  if (APPLE)
    target_link_libraries(${target} PRIVATE
      "-Wl,-exported_symbols_list,${CMAKE_CURRENT_SOURCE_DIR}/cpp/${list}.txt"
      "-Wl,-dead_strip")
  elseif (UNIX)
    target_link_libraries(${target} PRIVATE
      "-Wl,--version-script=${CMAKE_CURRENT_SOURCE_DIR}/cpp/${list}.map"
      "-Wl,--gc-sections")
  endif ()
endfunction()

# ---------------------------------------------------------------------------
# Libraries
# ---------------------------------------------------------------------------

# Common part used by decoder and encoder. Contains the Rust implementation.
add_library(woff2common
            cpp/common.cc)
add_dependencies(woff2common woff2_rs_build)
set_property(TARGET woff2common APPEND PROPERTY LINK_DEPENDS "${WOFF2_RS_LIB}")
if (BUILD_SHARED_LIBS)
  # Make sure the entry points used by libwoff2dec/libwoff2enc are kept.
  set(WOFF2_RS_ENTRY_POINTS
      woff2_rs_compute_final_size woff2_rs_convert_woff2_to_ttf
      woff2_rs_max_compressed_size woff2_rs_compressed_buffer_size
      woff2_rs_convert_ttf_to_woff2)
  foreach(sym ${WOFF2_RS_ENTRY_POINTS})
    if (APPLE)
      target_link_libraries(woff2common PRIVATE "-Wl,-u,_${sym}")
    else ()
      target_link_libraries(woff2common PRIVATE "-Wl,-u,${sym}")
    endif ()
  endforeach()
  target_link_libraries(woff2common PRIVATE "${WOFF2_RS_LIB}" ${WOFF2_RS_NATIVE_LIBS})
  woff2_export_api(woff2common exports)
else ()
  # Merge the Rust staticlib into libwoff2common.a.
  if (APPLE)
    add_custom_command(TARGET woff2common POST_BUILD
      COMMAND libtool -static -no_warning_for_no_symbols -o "$<TARGET_FILE:woff2common>.tmp"
              "$<TARGET_FILE:woff2common>" "${WOFF2_RS_LIB}"
      COMMAND "${CMAKE_COMMAND}" -E rename "$<TARGET_FILE:woff2common>.tmp"
              "$<TARGET_FILE:woff2common>"
      VERBATIM)
  elseif (MSVC)
    add_custom_command(TARGET woff2common POST_BUILD
      COMMAND lib.exe /NOLOGO "/OUT:$<TARGET_FILE:woff2common>.tmp"
              "$<TARGET_FILE:woff2common>" "${WOFF2_RS_LIB}"
      COMMAND "${CMAKE_COMMAND}" -E rename "$<TARGET_FILE:woff2common>.tmp"
              "$<TARGET_FILE:woff2common>"
      VERBATIM)
  else ()
    set(WOFF2_MERGE_SCRIPT "${CMAKE_CURRENT_BINARY_DIR}/merge_woff2common.mri")
    file(GENERATE OUTPUT "${WOFF2_MERGE_SCRIPT}" CONTENT
"create $<TARGET_FILE:woff2common>.tmp
addlib $<TARGET_FILE:woff2common>
addlib ${WOFF2_RS_LIB}
save
end
")
    add_custom_command(TARGET woff2common POST_BUILD
      COMMAND "${CMAKE_AR}" -M < "${WOFF2_MERGE_SCRIPT}"
      COMMAND "${CMAKE_COMMAND}" -E rename "$<TARGET_FILE:woff2common>.tmp"
              "$<TARGET_FILE:woff2common>"
      VERBATIM)
  endif ()
  target_link_libraries(woff2common INTERFACE ${WOFF2_RS_NATIVE_LIBS})
endif ()

# WOFF2 Decoder
add_library(woff2dec
            cpp/decode.cc
            cpp/output.cc)
target_link_libraries(woff2dec PUBLIC woff2common "${BROTLIDEC_LIBRARIES}")
woff2_export_api(woff2dec exports_cxx)

# WOFF2 Encoder
add_library(woff2enc
            cpp/encode.cc)
target_link_libraries(woff2enc PUBLIC woff2common "${BROTLIENC_LIBRARIES}")
woff2_export_api(woff2enc exports_cxx)

foreach(lib woff2common woff2dec woff2enc)
  set_target_properties(${lib} PROPERTIES
    SOVERSION ${WOFF2_VERSION}
    VERSION ${WOFF2_VERSION}
    POSITION_INDEPENDENT_CODE TRUE)
endforeach()

# ---------------------------------------------------------------------------
# Command line tools (Rust)
# ---------------------------------------------------------------------------

get_filename_component(BROTLIENC_LIBDIR "${BROTLIENC_LIBRARIES}" DIRECTORY)
set(WOFF2_EXECUTABLES woff2_compress woff2_decompress woff2_info)
set(WOFF2_EXECUTABLE_OUTPUTS "")
foreach(exe ${WOFF2_EXECUTABLES})
  list(APPEND WOFF2_EXECUTABLE_OUTPUTS
       "${CMAKE_CURRENT_BINARY_DIR}/${exe}${CMAKE_EXECUTABLE_SUFFIX}")
endforeach()
set(WOFF2_COPY_EXECUTABLES "")
foreach(exe ${WOFF2_EXECUTABLES})
  list(APPEND WOFF2_COPY_EXECUTABLES
       COMMAND "${CMAKE_COMMAND}" -E copy
               "${CARGO_TARGET_DIR}/bin/${CARGO_OUT_DIR}/${exe}${CMAKE_EXECUTABLE_SUFFIX}"
               "${CMAKE_CURRENT_BINARY_DIR}/${exe}${CMAKE_EXECUTABLE_SUFFIX}")
endforeach()
add_custom_command(
  OUTPUT ${WOFF2_EXECUTABLE_OUTPUTS}
  COMMAND "${CMAKE_COMMAND}" -E env "CARGO_TARGET_DIR=${CARGO_TARGET_DIR}/bin"
          "WOFF2_BROTLI_LIB_DIR=${BROTLIENC_LIBDIR}"
          "${CARGO}" build ${CARGO_FLAGS} --bins
          --no-default-features --features "${WOFF2_BIN_FEATURES}"
  ${WOFF2_COPY_EXECUTABLES}
  DEPENDS ${WOFF2_RS_SOURCES}
  COMMENT "Building woff2_compress, woff2_decompress and woff2_info (cargo)"
  USES_TERMINAL
  VERBATIM)
add_custom_target(woff2_executables ALL DEPENDS ${WOFF2_EXECUTABLE_OUTPUTS})
foreach(exe ${WOFF2_EXECUTABLES})
  add_custom_target(${exe})
  add_dependencies(${exe} woff2_executables)
endforeach()

# Fuzzer libraries
add_library(convert_woff2ttf_fuzzer STATIC cpp/convert_woff2ttf_fuzzer.cc)
target_link_libraries(convert_woff2ttf_fuzzer woff2dec)
add_library(convert_woff2ttf_fuzzer_new_entry STATIC cpp/convert_woff2ttf_fuzzer_new_entry.cc)
target_link_libraries(convert_woff2ttf_fuzzer_new_entry woff2dec)

# PC files
include(CMakeParseArguments)

function(generate_pkg_config_path outvar path)
  string(LENGTH "${path}" path_length)

  set(path_args ${ARGV})
  list(REMOVE_AT path_args 0 1)
  list(LENGTH path_args path_args_remaining)

  set("${outvar}" "${path}")

  while(path_args_remaining GREATER 1)
    list(GET path_args 0 name)
    list(GET path_args 1 value)

    get_filename_component(value_full "${value}" ABSOLUTE)
    string(LENGTH "${value}" value_length)

    if(path_length EQUAL value_length AND path STREQUAL value)
      set("${outvar}" "\${${name}}")
      break()
    elseif(path_length GREATER value_length)
      # We might be in a subdirectory of the value, but we have to be
      # careful about a prefix matching but not being a subdirectory
      # (for example, /usr/lib64 is not a subdirectory of /usr/lib).
      # We'll do this by making sure the next character is a directory
      # separator.
      string(SUBSTRING "${path}" ${value_length} 1 sep)
      if(sep STREQUAL "/")
        string(SUBSTRING "${path}" 0 ${value_length} s)
        if(s STREQUAL value)
          string(SUBSTRING "${path}" "${value_length}" -1 suffix)
          set("${outvar}" "\${${name}}${suffix}")
          break()
        endif()
      endif()
    endif()

    list(REMOVE_AT path_args 0 1)
    list(LENGTH path_args path_args_remaining)
  endwhile()

  set("${outvar}" "${${outvar}}" PARENT_SCOPE)
endfunction(generate_pkg_config_path)

function(generate_pkg_config output_file)
  set (options)
  set (oneValueArgs NAME DESCRIPTION URL VERSION PREFIX LIBDIR INCLUDEDIR LIBS_PRIVATE)
  set (multiValueArgs DEPENDS DEPENDS_PRIVATE CFLAGS LIBRARIES)
  cmake_parse_arguments(GEN_PKG "${options}" "${oneValueArgs}" "${multiValueArgs}" ${ARGN})
  unset (options)
  unset (oneValueArgs)
  unset (multiValueArgs)

  if(NOT GEN_PKG_PREFIX)
    set(GEN_PKG_PREFIX "${CMAKE_INSTALL_PREFIX}")
  endif()

  if(NOT GEN_PKG_LIBDIR)
    set(GEN_PKG_LIBDIR "${CMAKE_INSTALL_FULL_LIBDIR}")
  endif()
  generate_pkg_config_path(GEN_PKG_LIBDIR "${GEN_PKG_LIBDIR}"
    prefix "${GEN_PKG_PREFIX}")

  if(NOT GEN_PKG_INCLUDEDIR)
    set(GEN_PKG_INCLUDEDIR "${CMAKE_INSTALL_FULL_INCLUDEDIR}")
  endif()
  generate_pkg_config_path(GEN_PKG_INCLUDEDIR "${GEN_PKG_INCLUDEDIR}"
    prefix "${GEN_PKG_PREFIX}")

  file(WRITE  "${output_file}" "prefix=${GEN_PKG_PREFIX}\n")
  file(APPEND "${output_file}" "libdir=${GEN_PKG_LIBDIR}\n")
  file(APPEND "${output_file}" "includedir=${GEN_PKG_INCLUDEDIR}\n")
  file(APPEND "${output_file}" "\n")

  if(GEN_PKG_NAME)
    file(APPEND "${output_file}" "Name: ${GEN_PKG_NAME}\n")
  else()
    file(APPEND "${output_file}" "Name: ${CMAKE_PROJECT_NAME}\n")
  endif()

  if(GEN_PKG_DESCRIPTION)
    file(APPEND "${output_file}" "Description: ${GEN_PKG_DESCRIPTION}\n")
  endif()

  if(GEN_PKG_URL)
    file(APPEND "${output_file}" "URL: ${GEN_PKG_URL}\n")
  endif()

  if(GEN_PKG_VERSION)
    file(APPEND "${output_file}" "Version: ${GEN_PKG_VERSION}\n")
  endif()

  if(GEN_PKG_DEPENDS)
    file(APPEND "${output_file}" "Requires: ${GEN_PKG_DEPENDS}\n")
  endif()

  if(GEN_PKG_DEPENDS_PRIVATE)
    file(APPEND "${output_file}" "Requires.private:")
    foreach(lib ${GEN_PKG_DEPENDS_PRIVATE})
      file(APPEND "${output_file}" " ${lib}")
    endforeach()
    file(APPEND "${output_file}" "\n")
  endif()

  if(GEN_PKG_LIBRARIES)
    set(libs)

    file(APPEND "${output_file}" "Libs: -L\${libdir}")
    foreach(lib ${GEN_PKG_LIBRARIES})
      file(APPEND "${output_file}" " -l${lib}")
    endforeach()
    file(APPEND "${output_file}" "\n")
  endif()

  if(GEN_PKG_LIBS_PRIVATE)
    file(APPEND "${output_file}" "Libs.private:${GEN_PKG_LIBS_PRIVATE}\n")
  endif()

  file(APPEND "${output_file}" "Cflags: -I\${includedir}")
  if(GEN_PKG_CFLAGS)
    foreach(cflag ${GEN_PKG_CFLAGS})
      file(APPEND "${output_file}" " ${cflag}")
    endforeach()
  endif()
  file(APPEND "${output_file}" "\n")
endfunction(generate_pkg_config)

if (BUILD_SHARED_LIBS)
  set(WOFF2COMMON_LIBS_PRIVATE "")
else ()
  # Static libwoff2common contains the Rust standard library.
  set(WOFF2COMMON_LIBS_PRIVATE "${WOFF2_RS_PC_LIBS_PRIVATE}")
endif ()

generate_pkg_config ("${CMAKE_CURRENT_BINARY_DIR}/libwoff2common.pc"
  NAME libwoff2common
  DESCRIPTION "Shared data used by libwoff2 and libwoff2dec libraries"
  URL "https://github.com/dloebl/woof2"
  VERSION "${WOFF2_VERSION}"
  LIBRARIES woff2common
  LIBS_PRIVATE "${WOFF2COMMON_LIBS_PRIVATE}")

generate_pkg_config ("${CMAKE_CURRENT_BINARY_DIR}/libwoff2dec.pc"
  NAME libwoff2dec
  DESCRIPTION "WOFF2 decoder library"
  URL "https://github.com/dloebl/woof2"
  VERSION "${WOFF2_VERSION}"
  DEPENDS libbrotlidec
  DEPENDS_PRIVATE libwoff2common
  LIBRARIES woff2dec)

generate_pkg_config ("${CMAKE_CURRENT_BINARY_DIR}/libwoff2enc.pc"
  NAME libwoff2enc
  DESCRIPTION "WOFF2 encoder library"
  URL "https://github.com/dloebl/woof2"
  VERSION "${WOFF2_VERSION}"
  DEPENDS libbrotlienc
  DEPENDS_PRIVATE libwoff2common
  LIBRARIES woff2enc)

# Installation
if (NOT BUILD_SHARED_LIBS)
  install(
    PROGRAMS ${WOFF2_EXECUTABLE_OUTPUTS}
    DESTINATION "${CMAKE_INSTALL_BINDIR}"
  )
endif()

install(
  TARGETS woff2common woff2dec woff2enc
  ARCHIVE DESTINATION "${CMAKE_INSTALL_LIBDIR}"
  LIBRARY DESTINATION "${CMAKE_INSTALL_LIBDIR}"
  RUNTIME DESTINATION "${CMAKE_INSTALL_BINDIR}"
)
install(
  DIRECTORY include/woff2
  DESTINATION "${CMAKE_INSTALL_INCLUDEDIR}"
)
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/libwoff2common.pc"
  DESTINATION "${CMAKE_INSTALL_LIBDIR}/pkgconfig")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/libwoff2dec.pc"
  DESTINATION "${CMAKE_INSTALL_LIBDIR}/pkgconfig")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/libwoff2enc.pc"
  DESTINATION "${CMAKE_INSTALL_LIBDIR}/pkgconfig")
