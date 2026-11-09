cmake_minimum_required(VERSION 3.16)
project(mpfree LANGUAGES CXX VERSION 0.1.0)

# No MPC SDK or factory binaries. This builds only host-side isolated components.
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

add_library(mpfree-core STATIC src/core/ClickEngine.cpp)
target_include_directories(mpfree-core PUBLIC src/core/include)
target_compile_options(mpfree-core PRIVATE -Wall -Wextra -Wpedantic)

add_executable(mpfree-render src/desktop/render_click_wav.cpp)
target_link_libraries(mpfree-render PRIVATE mpfree-core)
target_compile_options(mpfree-render PRIVATE -Wall -Wextra -Wpedantic)

include(CTest)
if(BUILD_TESTING)
    add_executable(mpfree-core-test tests/core_click_test.cpp)
    target_link_libraries(mpfree-core-test PRIVATE mpfree-core)
    target_compile_options(mpfree-core-test PRIVATE -Wall -Wextra -Wpedantic)
    add_test(NAME core-click-engine COMMAND mpfree-core-test)
    add_test(NAME desktop-render-smoke
             COMMAND mpfree-render ${CMAKE_CURRENT_BINARY_DIR}/mpfree-test.wav 1 120)
endif()
