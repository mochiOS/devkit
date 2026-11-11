cmake_minimum_required(VERSION 3.20)
project(MochiOSSDKProbe LANGUAGES C CXX)
if(NOT CMAKE_SYSTEM_NAME STREQUAL "mochiOS" OR NOT CMAKE_CROSSCOMPILING)
    message(FATAL_ERROR "A real mochiOS cross toolchain is required")
endif()

# Bundle declarations must agree with both the DSO import surface and the
# process-owned runtime. These probes do not claim native resource access.
if(EXISTS "${CMAKE_SYSROOT}/include/mochi/bundle.h")
    add_library(sdk_bundle_probe_shared SHARED bundle.c)
    target_compile_features(sdk_bundle_probe_shared PRIVATE c_std_11)
    add_executable(sdk_bundle_probe bundle-main.cpp)
    target_compile_features(sdk_bundle_probe PRIVATE cxx_std_23)
    target_link_libraries(sdk_bundle_probe PRIVATE sdk_bundle_probe_shared)
endif()
if(NOT CMAKE_SYSROOT OR NOT IS_DIRECTORY "${CMAKE_SYSROOT}")
    message(FATAL_ERROR "SDK must provide a target sysroot")
endif()
if(NOT CMAKE_SIZEOF_VOID_P EQUAL 8 OR NOT CMAKE_SYSTEM_PROCESSOR MATCHES "^(x86_64|amd64|AMD64)$")
    message(FATAL_ERROR "Expected an x86-64 target")
endif()
foreach(mode LIBRARY INCLUDE PACKAGE)
    if(NOT CMAKE_FIND_ROOT_PATH_MODE_${mode} STREQUAL "ONLY")
        message(FATAL_ERROR "Host ${mode} search must be disabled")
    endif()
endforeach()
if(NOT CMAKE_FIND_ROOT_PATH_MODE_PROGRAM STREQUAL "NEVER")
    message(FATAL_ERROR "Build generators must run on the host")
endif()
set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
find_package(Threads REQUIRED)
add_library(sdk_probe_shared SHARED shared.cpp)
target_link_libraries(sdk_probe_shared PRIVATE Threads::Threads)
add_executable(sdk_probe main.cpp)
target_link_libraries(sdk_probe PRIVATE sdk_probe_shared Threads::Threads)
if(LIBRARY_PROBE)
    add_executable(library_probe "${LIBRARY_PROBE}")
    target_include_directories(library_probe PRIVATE /work/stage/usr/include /deps/usr/include)
    target_link_directories(library_probe PRIVATE /work/stage/usr/lib /deps/usr/lib)
    target_link_libraries(library_probe PRIVATE ${LIBRARY_LINKS})
endif()
# Older SDKs do not advertise the resolver API. Once the target header is
# installed, both executable exports and shared-library imports must link.
if(EXISTS "${CMAKE_SYSROOT}/include/resolv.h")
    add_library(sdk_resolver_probe_shared SHARED resolver.c)
    add_executable(sdk_resolver_probe resolver-main.c)
    target_link_libraries(sdk_resolver_probe PRIVATE sdk_resolver_probe_shared)
endif()

# An advertised target loader must be callable from both a process and a DSO.
# These are compile/link probes; native execution additionally needs signed
# package declarations and the matching OS broker/loader implementation.
if(EXISTS "${CMAKE_SYSROOT}/include/dlfcn.h")
    add_library(sdk_loader_probe_shared SHARED loader.c)
    add_library(sdk_loader_probe_private SHARED loader-private.c)
    add_executable(sdk_loader_probe loader-main.c)
    target_link_libraries(sdk_loader_probe PRIVATE sdk_loader_probe_shared)
endif()

# Real stack observations are required by JavaScriptCore. Both the DSO import
# and executable export must resolve; native tests additionally need kernel 706.
add_library(sdk_stack_probe_shared SHARED pthread-stack.c)
target_link_libraries(sdk_stack_probe_shared PRIVATE Threads::Threads)
add_executable(sdk_stack_probe pthread-stack-main.c)
target_link_libraries(sdk_stack_probe PRIVATE sdk_stack_probe_shared Threads::Threads)

# Current resident usage must be observed, not inferred from peak RSS.
add_library(sdk_memory_probe_shared SHARED process-memory.c)
add_executable(sdk_memory_probe process-memory-main.c)
target_link_libraries(sdk_memory_probe PRIVATE sdk_memory_probe_shared)

# Match the native runtime signatures before accepting a target SDK.
add_library(sdk_io_abi_probe_shared SHARED io-abi.c)
add_executable(sdk_io_abi_probe io-abi-main.c)
target_link_libraries(sdk_io_abi_probe PRIVATE sdk_io_abi_probe_shared)

# Unix extensions must resolve in both the DSO and executable process ABI.
if(EXISTS "${CMAKE_SYSROOT}/include/link.h")
add_library(sdk_unix_extensions_probe_shared SHARED unix-extensions.c)
target_link_libraries(sdk_unix_extensions_probe_shared PRIVATE Threads::Threads)
add_executable(sdk_unix_extensions_probe unix-extensions-main.c)
target_link_libraries(sdk_unix_extensions_probe PRIVATE sdk_unix_extensions_probe_shared Threads::Threads)
endif()
