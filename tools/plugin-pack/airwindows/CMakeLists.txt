# MPFree's CLAP wrapper for Airwindows effects (see airwindows_clap.cpp).
# tools/plugin-pack/build.py configures this with the fetched checkouts:
#   -DAIRWIN_SRC=<airwin2rack checkout> -DCLAP_SRC=<clap checkout>
cmake_minimum_required(VERSION 3.16)
project(mpfree-airwindows-clap LANGUAGES C CXX)

set(AIRWIN_SRC "" CACHE PATH "Airwindows Consolidated (airwin2rack) checkout")
set(CLAP_SRC "" CACHE PATH "CLAP headers checkout")
set(AW_COLLECTION "Recommended" CACHE STRING "Airwindows Consolidated collection to expose")
set(AW_VERSION "0" CACHE STRING "Version string the plugins report")
if(NOT EXISTS "${AIRWIN_SRC}/src/AirwinRegistry.h" OR NOT EXISTS "${CLAP_SRC}/include/clap/clap.h")
  message(FATAL_ERROR "Set AIRWIN_SRC and CLAP_SRC to the fetched checkouts")
endif()

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

# Airwindows' own registry library: every effect, its parameters and texts.
add_subdirectory("${AIRWIN_SRC}" airwin EXCLUDE_FROM_ALL)

add_library(airwindows MODULE airwindows_clap.cpp)
target_include_directories(airwindows PRIVATE "${CLAP_SRC}/include")
target_compile_definitions(airwindows PRIVATE
  MPFREE_AW_COLLECTION="${AW_COLLECTION}" MPFREE_AW_VERSION="${AW_VERSION}")
target_link_libraries(airwindows PRIVATE airwin-registry)
set_target_properties(airwindows PROPERTIES PREFIX "" SUFFIX ".clap"
  CXX_VISIBILITY_PRESET hidden VISIBILITY_INLINES_HIDDEN ON)
