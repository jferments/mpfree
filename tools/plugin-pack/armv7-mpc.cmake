# CMake toolchain for the MPC One: 32-bit ARMv7, hard-float, NEON/VFPv4,
# glibc 2.39 (Ubuntu noble's arm-linux-gnueabihf cross compiler links 2.39,
# the release the MPC reports; docs/dev/hardware/abi-handoff-survey.md).
#
# A plugin needs nothing from the MPC but its C library: the C++ library and
# libgcc are linked in, and every other library is linked statically or not at
# all. build.py checks each .clap file for that.
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR armv7l)

set(CMAKE_C_COMPILER arm-linux-gnueabihf-gcc)
set(CMAKE_CXX_COMPILER arm-linux-gnueabihf-g++)
set(CMAKE_ASM_COMPILER arm-linux-gnueabihf-gcc)
set(CMAKE_STRIP arm-linux-gnueabihf-strip)

set(MPC_FLAGS "-march=armv7-a -mfpu=neon-vfpv4 -mfloat-abi=hard -mthumb")
set(CMAKE_C_FLAGS_INIT "${MPC_FLAGS}")
set(CMAKE_CXX_FLAGS_INIT "${MPC_FLAGS}")
set(CMAKE_SHARED_LINKER_FLAGS_INIT "-static-libstdc++ -static-libgcc -Wl,--no-undefined -Wl,--exclude-libs,ALL -Wl,-z,relro,-z,now")
# A plugin (a MODULE library) exports clap_entry and nothing else.
set(CMAKE_MODULE_LINKER_FLAGS_INIT "${CMAKE_SHARED_LINKER_FLAGS_INIT} -Wl,--version-script=${CMAKE_CURRENT_LIST_DIR}/clap-exports.map")

# Libraries and headers from the armhf packages, programs from the build machine.
set(CMAKE_FIND_ROOT_PATH /usr/arm-linux-gnueabihf /usr/lib/arm-linux-gnueabihf)
set(CMAKE_LIBRARY_ARCHITECTURE arm-linux-gnueabihf)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY BOTH)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE BOTH)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE BOTH)
set(ENV{PKG_CONFIG_LIBDIR} "/usr/lib/arm-linux-gnueabihf/pkgconfig:/usr/share/pkgconfig")
set(ENV{PKG_CONFIG_PATH} "")
