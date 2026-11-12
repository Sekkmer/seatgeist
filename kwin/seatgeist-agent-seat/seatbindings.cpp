// SPDX-License-Identifier: MIT OR Apache-2.0
#include "seatbindings.h"

#include <wayland/pointer.h>
#include <wayland/clientconnection.h>
#include <wayland/seat.h>
#include <wayland/surface.h>
#include <wayland/pointerconstraints_v1.h>

#include <cstring>
#include <cmath>

namespace KWin
{
SeatBindings::SeatBindings(wl_display *display)
    : m_logger(wl_display_add_protocol_logger(display, protocolMessage, this))
{
}

SeatBindings::~SeatBindings()
{
    m_pointerObjects.clear();
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
    m_pointerObjects.remove_if([seat](const auto &binding) { return binding->seat == seat; });
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

SeatBindings::PointerBinding::~PointerBinding()
{
    wl_list_remove(&pointerDestroy.listener.link);
    if (objectObserved) { wl_list_remove(&objectDestroy.listener.link); }
}

wl_resource *SeatBindings::PointerBinding::object()
{
    auto *resource = wl_client_get_object(client, id);
    const char *expected = lock ? "zwp_locked_pointer_v1" : "zwp_relative_pointer_v1";
    if (!resource || std::strcmp(wl_resource_get_class(resource), expected) != 0) { return nullptr; }
    if (!objectObserved) {
        objectDestroy.binding = this;
        objectDestroy.listener.notify = SeatBindings::pointerObjectDestroyed;
        wl_resource_add_destroy_listener(resource, &objectDestroy.listener);
        objectObserved = true;
    }
    return resource;
}

void SeatBindings::rememberPointerObject(wl_resource *resource, uint32_t id, SurfaceInterface *surface, bool lock)
{
    auto *pointer = PointerInterface::get(resource);
    if (!pointer || !m_seats.contains(pointer->seat())) { return; }
    auto binding = std::make_unique<PointerBinding>();
    binding->owner = this;
    binding->seat = pointer->seat();
    binding->client = wl_resource_get_client(resource);
    binding->id = id;
    binding->lock = lock;
    binding->surface = surface;
    binding->pointerDestroy.binding = binding.get();
    binding->pointerDestroy.listener.notify = pointerObjectDestroyed;
    wl_resource_add_destroy_listener(resource, &binding->pointerDestroy.listener);
    m_pointerObjects.push_back(std::move(binding));
}

void SeatBindings::pointerObjectDestroyed(wl_listener *listener, void *)
{
    auto *link = reinterpret_cast<PointerBinding::Link *>(listener);
    auto *binding = link->binding;
    binding->owner->m_pointerObjects.remove_if([binding](const auto &entry) { return entry.get() == binding; });
}

bool SeatBindings::relativePointerReady(SeatInterface *seat, wl_client *client)
{
    for (const auto &binding : m_pointerObjects) {
        if (!binding->lock && binding->seat == seat && binding->client == client && binding->object()) { return true; }
    }
    return false;
}

bool SeatBindings::lockedPointerReady(SeatInterface *seat, wl_client *client, SurfaceInterface *surface)
{
    if (!surface || seat->focusedPointerSurface() != surface || seat->pointer()->focusedSurface() != surface
        || surface->client()->client() != client || !surface->lockedPointer() || !surface->lockedPointer()->isLocked()) { return false; }
    for (const auto &binding : m_pointerObjects) {
        if (binding->lock && binding->seat == seat && binding->client == client && binding->surface == surface && binding->object()) { return true; }
    }
    return false;
}

bool SeatBindings::deliverRelativePointer(SeatInterface *seat, wl_client *client, SurfaceInterface *surface,
                                          const QPointF &delta, std::chrono::microseconds timestamp)
{
    if (!std::isfinite(delta.x()) || !std::isfinite(delta.y()) || std::abs(delta.x()) > 4096
        || std::abs(delta.y()) > 4096 || delta.isNull()
        || !lockedPointerReady(seat, client, surface) || !relativePointerReady(seat, client)) { return false; }
    seat->setTimestamp(timestamp);
    seat->relativePointerMotion(delta, delta, timestamp);
    seat->notifyPointerFrame();
    return true;
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
        const char *request = message->message->name;
        if (std::strcmp(interface, "zwp_relative_pointer_manager_v1") == 0 && std::strcmp(request, "get_relative_pointer") == 0) {
            self.rememberPointerObject(reinterpret_cast<wl_resource *>(message->arguments[1].o), message->arguments[0].n, nullptr, false);
        } else if (std::strcmp(interface, "zwp_pointer_constraints_v1") == 0 && std::strcmp(request, "lock_pointer") == 0) {
            auto *surface = SurfaceInterface::get(reinterpret_cast<wl_resource *>(message->arguments[1].o));
            self.rememberPointerObject(reinterpret_cast<wl_resource *>(message->arguments[2].o), message->arguments[0].n, surface, true);
        } else if ((std::strcmp(interface, "zwp_relative_pointer_v1") == 0 || std::strcmp(interface, "zwp_locked_pointer_v1") == 0)
                   && std::strcmp(request, "destroy") == 0) {
            // Includes objects destroyed before the first readiness query and
            // before a destroy listener could be attached. ID reuse is fresh.
            auto *client = wl_resource_get_client(message->resource);
            const auto id = wl_resource_get_id(message->resource);
            self.m_pointerObjects.remove_if([client, id](const auto &binding) { return binding->client == client && binding->id == id; });
        }

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
