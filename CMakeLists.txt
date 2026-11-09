cmake_minimum_required(VERSION 3.20)
project(mpfree LANGUAGES CXX)

add_library(mpfree_sequencer STATIC
    engine/src/sequencer.cpp
)
target_include_directories(mpfree_sequencer PUBLIC engine/include)
target_compile_features(mpfree_sequencer PUBLIC cxx_std_20)

if(CMAKE_CXX_COMPILER_ID MATCHES "Clang|GNU")
    target_compile_options(mpfree_sequencer PRIVATE -Wall -Wextra -Wpedantic)
endif()

enable_testing()
add_executable(mpfree_sequencer_test engine/tests/sequencer_test.cpp)
target_link_libraries(mpfree_sequencer_test PRIVATE mpfree_sequencer)
add_test(NAME sequencer COMMAND mpfree_sequencer_test)

add_executable(mpfree_demo engine/examples/render_demo.cpp)
target_link_libraries(mpfree_demo PRIVATE mpfree_sequencer)
