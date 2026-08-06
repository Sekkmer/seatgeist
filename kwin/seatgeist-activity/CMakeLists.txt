cmake_minimum_required(VERSION 3.24)

project(seatgeist_kwin_activity LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 REQUIRED COMPONENTS Core DBus Gui Widgets)
find_package(KF6Config REQUIRED)
find_package(KF6WindowSystem REQUIRED)
include(GNUInstallDirs)

find_path(KWIN_INCLUDE_DIR plugin.h PATH_SUFFIXES kwin REQUIRED)
find_library(KWIN_LIBRARY NAMES kwin REQUIRED)

file(READ "${KWIN_INCLUDE_DIR}/config-kwin.h" KWIN_CONFIG_HEADER)
string(
    REGEX MATCH
    "#define KWIN_PLUGIN_VERSION_STRING \"([0-9]+\\.[0-9]+\\.[0-9]+)\""
    KWIN_PLUGIN_VERSION_MATCH
    "${KWIN_CONFIG_HEADER}"
)
if(NOT CMAKE_MATCH_1)
    message(FATAL_ERROR "Could not read KWIN_PLUGIN_VERSION_STRING from config-kwin.h")
endif()
set(SEATGEIST_KWIN_PLUGIN_VERSION "${CMAKE_MATCH_1}")
configure_file(
    kwin_plugin_abi.h.in
    generated/kwin_plugin_abi.h
    @ONLY
)

add_library(seatgeistactivity MODULE
    activityplugin.cpp
    activityplugin.h
    ${CMAKE_CURRENT_BINARY_DIR}/generated/kwin_plugin_abi.h
    main.cpp
)

set_target_properties(seatgeistactivity PROPERTIES PREFIX "")

target_include_directories(seatgeistactivity PRIVATE
    ${KWIN_INCLUDE_DIR}
    ${CMAKE_CURRENT_BINARY_DIR}/generated
)

target_link_libraries(seatgeistactivity PRIVATE
    ${KWIN_LIBRARY}
    Qt6::Core
    Qt6::DBus
    Qt6::Gui
    Qt6::Widgets
    KF6::ConfigCore
    KF6::WindowSystem
)

install(TARGETS seatgeistactivity
    LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR}/qt6/plugins/kwin/plugins
)
