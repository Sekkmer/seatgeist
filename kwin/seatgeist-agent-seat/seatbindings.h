// SPDX-License-Identifier: MIT OR Apache-2.0
#pragma once

#include <wayland-server-core.h>

#include <list>
#include <memory>
#include <set>

namespace KWin
{
class SeatInterface;

// Track actual client bindings, not just the compositor's focused surface.
// No key contents or other application protocol traffic are retained.
class SeatBindings
{
public:
    explicit SeatBindings(wl_display *display);
    ~SeatBindings();
    void watch(SeatInterface *seat);
    void forget(SeatInterface *seat);
    bool available() const;
    bool keyboardReady(SeatInterface *seat, wl_client *client) const;
    bool pointerReady(SeatInterface *seat, wl_client *client) const;

private:
    struct KeyboardBinding {
        wl_listener destroy;
        SeatBindings *owner;
        SeatInterface *seat;
        wl_client *client;
        wl_resource *resource;
    };
    static void protocolMessage(void *, wl_protocol_logger_type, const wl_protocol_logger_message *);
    static void resourceDestroyed(wl_listener *, void *);
    wl_protocol_logger *m_logger;
    std::set<SeatInterface *> m_seats;
    std::list<std::unique_ptr<KeyboardBinding>> m_keyboards;
    SeatInterface *m_requestedSeat = nullptr;
    wl_client *m_requestedClient = nullptr;
    uint32_t m_requestedKeyboard = 0;
};
}
