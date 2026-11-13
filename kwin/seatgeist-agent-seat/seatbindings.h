// SPDX-License-Identifier: MIT OR Apache-2.0
#pragma once

#include <wayland-server-core.h>
#include <QPointer>
#include <QPointF>
#include <QObject>
#include <chrono>

#include <list>
#include <memory>
#include <set>
#include <map>
#include <vector>

namespace KWin
{
class SeatInterface;
class SurfaceInterface;
class LockedPointerV1Interface;

// Track actual client bindings, not just the compositor's focused surface.
// No key contents or other application protocol traffic are retained.
class SeatBindings : public QObject
{
public:
    struct LockReleaseObservation {
        QPointer<LockedPointerV1Interface> lock;
        bool clientDestroyed = false;
    };
    explicit SeatBindings(wl_display *display, SeatInterface *physicalSeat = nullptr);
    ~SeatBindings();
    void watch(SeatInterface *seat);
    void forget(SeatInterface *seat);
    bool available() const;
    bool keyboardReady(SeatInterface *seat, wl_client *client) const;
    bool pointerReady(SeatInterface *seat, wl_client *client) const;
    bool relativePointerReady(SeatInterface *seat, wl_client *client);
    bool lockedPointerRequested(SeatInterface *seat, wl_client *client, SurfaceInterface *surface);
    bool pointerLockRegionContains(SeatInterface *seat, SurfaceInterface *surface) const;
    // Only the policy-authorized lane target may activate a client constraint.
    // This never enters/focuses a surface or moves either seat's pointer.
    void setConstraintTarget(SeatInterface *seat, SurfaceInterface *surface);
    bool lockedPointerReady(SeatInterface *seat, wl_client *client, SurfaceInterface *surface);
    std::shared_ptr<LockReleaseObservation> observeActiveLock(SeatInterface *, wl_client *, SurfaceInterface *);
    bool deliverRelativePointer(SeatInterface *seat, wl_client *client, SurfaceInterface *surface,
                                const QPointF &delta, std::chrono::microseconds timestamp);

private:
    struct LockTarget;
    void reconcilePointerLock(SeatInterface *seat);
    void clearConstraintTarget(SeatInterface *seat);
    QPointer<SeatInterface> m_physicalSeat;
    std::map<SeatInterface *, std::unique_ptr<LockTarget>> m_lockTargets;
    struct KeyboardBinding {
        wl_listener destroy;
        SeatBindings *owner;
        SeatInterface *seat;
        wl_client *client;
        wl_resource *resource;
    };
    struct PointerBinding {
        struct Link { wl_listener listener; PointerBinding *binding; } pointerDestroy{}, objectDestroy{};
        ~PointerBinding();
        SeatBindings *owner;
        SeatInterface *seat;
        wl_client *client;
        uint32_t id;
        bool lock;
        bool objectObserved = false;
        QPointer<SurfaceInterface> surface;
        std::shared_ptr<LockReleaseObservation> release;
        wl_resource *object();
    };
    void rememberPointerObject(wl_resource *pointer, uint32_t id, SurfaceInterface *surface, bool lock);
    static void pointerObjectDestroyed(wl_listener *, void *);
    std::list<std::unique_ptr<PointerBinding>> m_pointerObjects;
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
