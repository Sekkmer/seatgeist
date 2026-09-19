// SPDX-License-Identifier: MIT OR Apache-2.0
#include "seatbindings.h"

#include <wayland/pointer.h>
#include <wayland/seat.h>

#include <cstring>

namespace KWin
{
SeatBindings::SeatBindings(wl_display *display)
    : m_logger(wl_display_add_protocol_logger(display, protocolMessage, this))
{
}

SeatBindings::~SeatBindings()
{
    if (m_logger) {
        wl_protocol_logger_destroy(m_logger);
    }
    for (const auto &binding : m_keyboards) {
        wl_list_remove(&binding->destroy.link);
    }
}

bool SeatBindings::available() const
{
    return m_logger != nullptr;
}

void SeatBindings::watch(SeatInterface *seat)
{
    m_seats.insert(seat);
}

void SeatBindings::forget(SeatInterface *seat)
{
    m_seats.erase(seat);
    if (m_requestedSeat == seat) {
        m_requestedSeat = nullptr;
    }
    m_keyboards.remove_if([seat](const auto &binding) {
        if (binding->seat != seat) {
            return false;
        }
        wl_list_remove(&binding->destroy.link);
        return true;
    });
}

bool SeatBindings::keyboardReady(SeatInterface *seat, wl_client *client) const
{
    for (const auto &binding : m_keyboards) {
        if (binding->seat == seat && binding->client == client) {
            return true;
        }
    }
    return false;
}

bool SeatBindings::pointerReady(SeatInterface *seat, wl_client *client) const
{
    struct Query {
        SeatInterface *seat;
        bool found = false;
    } query{seat};
    wl_client_for_each_resource(client, [](wl_resource *resource, void *data) {
        auto &query = *static_cast<Query *>(data);
        if (std::strcmp(wl_resource_get_class(resource), "wl_pointer") == 0) {
            auto *pointer = PointerInterface::get(resource);
            if (pointer && pointer->seat() == query.seat) {
                query.found = true;
                return WL_ITERATOR_STOP;
            }
        }
        return WL_ITERATOR_CONTINUE;
    }, &query);
    return query.found;
}

void SeatBindings::resourceDestroyed(wl_listener *listener, void *)
{
    // The listener is the first member of this standard-layout record.
    auto *binding = reinterpret_cast<KeyboardBinding *>(listener);
    wl_list_remove(&binding->destroy.link);
    binding->owner->m_keyboards.remove_if([binding](const auto &entry) {
        return entry.get() == binding;
    });
}

void SeatBindings::protocolMessage(
    void *data, wl_protocol_logger_type direction, const wl_protocol_logger_message *message)
{
    auto &self = *static_cast<SeatBindings *>(data);
    const char *interface = wl_resource_get_class(message->resource);
    if (direction == WL_PROTOCOL_LOGGER_REQUEST) {
        // KWin creates the keyboard and sends its configured keymap inside
        // get_keyboard's synchronous request handler. A new request clears
        // the correlation, so object-id reuse cannot inherit readiness.
        self.m_requestedSeat = nullptr;
        if (std::strcmp(interface, "wl_seat") != 0
            || std::strcmp(message->message->name, "get_keyboard") != 0) {
            return;
        }
        auto *seat = SeatInterface::get(message->resource);
        if (!self.m_seats.contains(seat)) {
            return;
        }
        self.m_requestedSeat = seat;
        self.m_requestedClient = wl_resource_get_client(message->resource);
        self.m_requestedKeyboard = message->arguments[0].n;
    } else if (self.m_requestedSeat && std::strcmp(interface, "wl_keyboard") == 0
               && std::strcmp(message->message->name, "keymap") == 0
               && wl_resource_get_client(message->resource) == self.m_requestedClient
               && wl_resource_get_id(message->resource) == self.m_requestedKeyboard) {
        auto binding = std::make_unique<KeyboardBinding>();
        binding->destroy.notify = resourceDestroyed;
        binding->owner = &self;
        binding->seat = self.m_requestedSeat;
        binding->client = self.m_requestedClient;
        binding->resource = message->resource;
        wl_resource_add_destroy_listener(binding->resource, &binding->destroy);
        self.m_keyboards.push_back(std::move(binding));
        self.m_requestedSeat = nullptr;
    }
}
}
