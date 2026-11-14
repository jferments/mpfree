# Included after each project() of a JUCE plugin (CMAKE_PROJECT_INCLUDE).
#
# JUCE links FreeType (and in JUCE 8 fontconfig) to draw text in an editor,
# which MPFree never opens, and the MPC may not have them. So prefer static
# libraries: the armhf packages' .a files are position-independent and go
# inside the .clap, with what they need in turn after everything else.
# build.py's checks fail a file that still needs any of them.
set(CMAKE_FIND_LIBRARY_SUFFIXES .a)
set(_mpc_lib /usr/lib/arm-linux-gnueabihf)
set(_mpc_static "")
foreach(_l png16 z brotlidec brotlicommon bz2 expat)
  list(APPEND _mpc_static "${_mpc_lib}/lib${_l}.a")
endforeach()
list(JOIN _mpc_static " " _mpc_static)
set(CMAKE_C_STANDARD_LIBRARIES "${_mpc_static} -lm")
set(CMAKE_CXX_STANDARD_LIBRARIES "${_mpc_static} -lm")
