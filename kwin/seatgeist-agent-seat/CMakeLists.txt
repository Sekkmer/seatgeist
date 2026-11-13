cmake_minimum_required(VERSION 3.24)

project(seatgeist_kwin_agent_seat LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 REQUIRED COMPONENTS Core DBus Gui Widgets)
find_package(KF6Config REQUIRED)
find_package(KF6WindowSystem REQUIRED)
include(GNUInstallDirs)

find_path(KWIN_INCLUDE_DIR plugin.h PATH_SUFFIXES kwin REQUIRED)
find_library(KWIN_LIBRARY NAMES kwin REQUIRED)
find_package(PkgConfig REQUIRED)
pkg_check_modules(XKBCOMMON REQUIRED IMPORTED_TARGET xkbcommon)
pkg_check_modules(WAYLAND_SERVER REQUIRED IMPORTED_TARGET wayland-server)

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

set(SEATGEIST_AGENT_SEAT_BUILD_INPUTS
    agentseatplugin.cpp
    agentseatplugin.h
    agentcursor.h
    seatbindings.cpp
    seatbindings.h
    eventbatch.cpp
    eventbatch.h
    altpointclick.cpp
    altpointclick.h
    gesturerunner.cpp
    gesturerunner.h
)
set_property(
    DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS
    ${SEATGEIST_AGENT_SEAT_BUILD_INPUTS}
)
set(SEATGEIST_AGENT_SEAT_BUILD_MATERIAL "")
foreach(BUILD_INPUT IN LISTS SEATGEIST_AGENT_SEAT_BUILD_INPUTS)
    file(SHA256 "${CMAKE_CURRENT_SOURCE_DIR}/${BUILD_INPUT}" BUILD_INPUT_SHA256)
    string(APPEND SEATGEIST_AGENT_SEAT_BUILD_MATERIAL "${BUILD_INPUT}:${BUILD_INPUT_SHA256}\n")
endforeach()
string(
    SHA256 SEATGEIST_AGENT_SEAT_BUILD_ID
    "${SEATGEIST_AGENT_SEAT_BUILD_MATERIAL}"
)
configure_file(
    agent_seat_build_id.h.in
    generated/agent_seat_build_id.h
    @ONLY
)

add_library(seatgeistagentseat MODULE
    agentseatplugin.cpp
    agentseatplugin.h
    agentcursor.h
    seatbindings.cpp
    seatbindings.h
    eventbatch.cpp
    eventbatch.h
    altpointclick.cpp
    altpointclick.h
    gesturerunner.cpp
    gesturerunner.h
    ${CMAKE_CURRENT_BINARY_DIR}/generated/agent_seat_build_id.h
    ${CMAKE_CURRENT_BINARY_DIR}/generated/kwin_plugin_abi.h
    main.cpp
)

set_target_properties(seatgeistagentseat PROPERTIES PREFIX "")

target_include_directories(seatgeistagentseat PRIVATE
    ${KWIN_INCLUDE_DIR}
    ${CMAKE_CURRENT_BINARY_DIR}/generated
)

target_link_libraries(seatgeistagentseat PRIVATE
    ${KWIN_LIBRARY}
    Qt6::Core
    Qt6::DBus
    Qt6::Gui
    Qt6::Widgets
    KF6::ConfigCore
    KF6::WindowSystem
    PkgConfig::XKBCOMMON
    PkgConfig::WAYLAND_SERVER
)

install(TARGETS seatgeistagentseat
    LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR}/qt6/plugins/kwin/plugins
)

include(CTest)
if(BUILD_TESTING)
    find_package(Qt6 REQUIRED COMPONENTS Test)
    pkg_check_modules(WAYLAND_CLIENT REQUIRED IMPORTED_TARGET wayland-client)
    enable_language(C)
    find_program(WAYLAND_SCANNER wayland-scanner REQUIRED)
    foreach(PROTOCOL relative-pointer pointer-constraints)
        set(PROTOCOL_XML "${CMAKE_CURRENT_SOURCE_DIR}/test-protocols/${PROTOCOL}-unstable-v1.xml")
        add_custom_command(OUTPUT "${CMAKE_CURRENT_BINARY_DIR}/${PROTOCOL}-client.h" "${CMAKE_CURRENT_BINARY_DIR}/${PROTOCOL}-code.c"
            COMMAND ${WAYLAND_SCANNER} client-header ${PROTOCOL_XML} "${CMAKE_CURRENT_BINARY_DIR}/${PROTOCOL}-client.h"
            COMMAND ${WAYLAND_SCANNER} private-code ${PROTOCOL_XML} "${CMAKE_CURRENT_BINARY_DIR}/${PROTOCOL}-code.c"
            DEPENDS ${PROTOCOL_XML} VERBATIM)
        list(APPEND POINTER_TEST_PROTOCOLS "${CMAKE_CURRENT_BINARY_DIR}/${PROTOCOL}-client.h" "${CMAKE_CURRENT_BINARY_DIR}/${PROTOCOL}-code.c")
    endforeach()
    add_executable(seatbindings_test seatbindings_test.cpp seatbindings.cpp altpointclick.cpp gesturerunner.cpp ${POINTER_TEST_PROTOCOLS})
    target_include_directories(seatbindings_test PRIVATE ${CMAKE_CURRENT_BINARY_DIR})
    target_include_directories(seatbindings_test PRIVATE ${KWIN_INCLUDE_DIR})
    target_link_libraries(seatbindings_test PRIVATE
        ${KWIN_LIBRARY} Qt6::Core Qt6::Gui Qt6::Test
        PkgConfig::WAYLAND_SERVER PkgConfig::WAYLAND_CLIENT PkgConfig::XKBCOMMON)
    add_test(NAME seatbindings COMMAND seatbindings_test)
    add_executable(eventbatch_test eventbatch_test.cpp eventbatch.cpp)
    target_link_libraries(eventbatch_test PRIVATE Qt6::Core Qt6::Test)
    add_test(NAME eventbatch COMMAND eventbatch_test)
    add_executable(gesturerunner_test gesturerunner_test.cpp gesturerunner.cpp)
    target_link_libraries(gesturerunner_test PRIVATE Qt6::Core Qt6::Test)
    add_test(NAME gesture_runner COMMAND gesturerunner_test)
    add_executable(focus_fixture focus_fixture.cpp)
    target_link_libraries(focus_fixture PRIVATE Qt6::Widgets PkgConfig::WAYLAND_CLIENT)
endif()
