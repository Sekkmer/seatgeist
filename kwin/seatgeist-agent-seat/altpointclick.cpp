// SPDX-License-Identifier: MIT OR Apache-2.0
#include "altpointclick.h"
#include <wayland/clientconnection.h>
#include <wayland/pointer.h>
#include <wayland/pointerconstraints_v1.h>
#include <wayland/seat.h>
#include <wayland/surface.h>

namespace KWin {
AltPointClick::AltPointClick(SeatBindings &bindings, SeatInterface *seat, SurfaceInterface *surface, Hooks hooks)
    : m_bindings(bindings), m_seat(seat), m_surface(surface), m_hooks(std::move(hooks)) {}
AltPointClick::~AltPointClick() { cancel(); }
bool AltPointClick::valid(qint64 now) const
{
    return now < m_deadline && m_surface && m_hooks.valid()
        && m_seat->focusedKeyboardSurface() == m_surface
        && m_seat->focusedPointerSurface() == m_surface
        && m_seat->pointer()->focusedSurface() == m_surface
        && m_bindings.keyboardReady(m_seat, m_surface->client()->client())
        && m_bindings.pointerReady(m_seat, m_surface->client()->client());
}
bool AltPointClick::unlocked() const
{
    // A compositor-only deactivation is insufficient. Observe the request to
    // destroy this exact seat/client's original lock and its actual removal.
    return m_surface && m_release && m_release->clientDestroyed
        && m_release->lock.isNull() && !m_surface->lockedPointer();
}
void AltPointClick::start(qint64 now)
{
    if (m_state != Initial) { return; }
    m_deadline = now + 1000;
    if (!valid(now)) { finish(false, QStringLiteral("Alt-point click target/binding unavailable")); return; }
    m_release = m_bindings.observeActiveLock(m_seat, m_surface->client()->client(), m_surface);
    if (!m_release) { finish(false, QStringLiteral("Alt-point click requires an active same-seat/client lock")); return; }
    m_state = WaitingUnlock;
    m_altDown = true;
    m_hooks.alt(true);
    // Always return to the compositor loop. No motion or buttons here.
}
void AltPointClick::tick(qint64 now)
{
    if (m_state == Initial || m_state == Terminal) { return; }
    if (!valid(now)) { finish(false, QStringLiteral("Alt-point click target lost, cancelled or unlock deadline expired")); return; }
    if (m_state == WaitingUnlock && unlocked()) {
        m_state = Authorizing;
        m_hooks.authorize(); // asynchronous, fresh daemon lease/action check
    }
}
void AltPointClick::authorized(bool allowed, qint64 now)
{
    if (m_state != Authorizing) { return; }
    if (!allowed || !valid(now) || !unlocked()) {
        finish(false, QStringLiteral("Alt-point click revalidation refused")); return;
    }
    const bool clicked = m_hooks.click();
    finish(clicked, clicked ? QString() : QStringLiteral("Alt-point click target changed before click"));
}
void AltPointClick::cancel(const QString &reason)
{
    if (m_state != Terminal) { finish(false, reason); }
}
void AltPointClick::finish(bool ok, const QString &reason)
{
    if (m_state == Terminal) { return; }
    m_state = Terminal;
    if (m_altDown) { m_altDown = false; m_hooks.alt(false); }
    m_hooks.finished(ok, reason);
}
}
