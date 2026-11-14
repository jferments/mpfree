# Included after each project() of a JUCE plugin (CMAKE_PROJECT_INCLUDE).
#
# JUCE links FreeType (and in JUCE 8 fontconfig) to draw text in an editor,
# which MPFree never opens, and the MPC may not have them. So they are linked
# in statically: JUCE finds them through pkg-config, and these cache entries
# answer that search with the armhf packages' .a files (position-independent).
# What those need in turn goes after everything else on the link line. The C
# library stays shared. build.py's checks fail a file that still needs any
# library but the C library.
set(_mpc_lib /usr/lib/arm-linux-gnueabihf)
set(pkgcfg_lib_juce_graphics_LINUX_DEPS_freetype "${_mpc_lib}/libfreetype.a" CACHE FILEPATH "FreeType, linked in")
set(pkgcfg_lib_juce_graphics_LINUX_DEPS_fontconfig "${_mpc_lib}/libfontconfig.a" CACHE FILEPATH "fontconfig, linked in")
set(_mpc_static "")
foreach(_l png16 z brotlidec brotlicommon bz2 expat)
  list(APPEND _mpc_static "${_mpc_lib}/lib${_l}.a")
endforeach()
list(JOIN _mpc_static " " _mpc_static)
set(CMAKE_C_STANDARD_LIBRARIES "${_mpc_static} -lm")
set(CMAKE_CXX_STANDARD_LIBRARIES "${_mpc_static} -lm")
