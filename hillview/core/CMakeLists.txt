cmake_minimum_required(VERSION 3.16)
# macOS: arm64 support starts at 11.0. Must be set before project(), where APPLE is not defined yet, so test
# the system names instead. Applies when building ON a Mac, or with -DCMAKE_SYSTEM_NAME=Darwin on the command line
# (a toolchain file is read later, inside project(); then pass -DCMAKE_OSX_DEPLOYMENT_TARGET yourself).
if((CMAKE_HOST_SYSTEM_NAME STREQUAL "Darwin" OR CMAKE_SYSTEM_NAME STREQUAL "Darwin") AND NOT CMAKE_OSX_DEPLOYMENT_TARGET)
  set(CMAKE_OSX_DEPLOYMENT_TARGET "11.0" CACHE STRING "Minimum macOS version")
endif()

project(hillview_core VERSION 0.1 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release)
endif()
set(CMAKE_CXX_VISIBILITY_PRESET hidden)
set(CMAKE_VISIBILITY_INLINES_HIDDEN ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

find_package(Threads REQUIRED)

add_library(hillview SHARED src/hv_api.cpp src/hv_kernels.cpp)
target_include_directories(hillview PUBLIC include)
target_link_libraries(hillview PRIVATE Threads::Threads)
# -O3 for the hot loops; no -ffast-math so results stay reproducible across builds.
# No -march=native: binaries must run on any x86-64. AVX2/FMA kernels are selected at load time (target_clones).
target_compile_options(hillview PRIVATE $<$<NOT:$<CXX_COMPILER_ID:MSVC>>:-O3 -fno-math-errno -fno-trapping-math -ffp-contract=off -fno-exceptions -fno-rtti -Wall -Wextra>)
set_target_properties(hillview PROPERTIES VERSION ${PROJECT_VERSION} SOVERSION 1 MACOSX_RPATH ON)
