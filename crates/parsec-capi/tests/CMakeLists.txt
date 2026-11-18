# SPDX-License-Identifier: Apache-2.0
#
# Builds tests/capi.c twice, against the shared and the static library, from
# an installed package found the way a user's project finds it:
#   cmake -S crates/parsec-capi/tests -B build -DCMAKE_PREFIX_PATH=<package>
#   cmake --build build --config Release
#   ctest --test-dir build -C Release --output-on-failure
cmake_minimum_required(VERSION 3.21)
project(parsec_capi_test C)

find_package(parsec 0.1 REQUIRED CONFIG)
enable_testing()

add_executable(capi_shared capi.c)
target_link_libraries(capi_shared PRIVATE parsec::parsec)
add_executable(capi_static capi.c)
target_link_libraries(capi_static PRIVATE parsec::parsec_static)

if(WIN32)
  # A program finds a DLL next to itself.
  add_custom_command(TARGET capi_shared POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy $<TARGET_RUNTIME_DLLS:capi_shared> $<TARGET_FILE_DIR:capi_shared>
    COMMAND_EXPAND_LISTS)
endif()

add_test(NAME shared COMMAND capi_shared)
add_test(NAME static COMMAND capi_static)
