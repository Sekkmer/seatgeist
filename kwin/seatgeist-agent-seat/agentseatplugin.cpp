// SPDX-License-Identifier: MIT OR Apache-2.0

#include "agentseatplugin.h"
#include "seatbindings.h"
#include "altpointclick.h"
#include <agent_seat_build_id.h>

#include <input.h>
#include <input_event.h>
#include <wayland/clientconnection.h>
#include <wayland/display.h>
#include <wayland/keyboard.h>
#include <wayland/pointerconstraints_v1.h>
#include <wayland/seat.h>
#include <wayland/surface.h>
#include <wayland_server.h>
#include <window.h>
#include <workspace.h>

#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QDBusPendingCall>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>
#include <QDBusServiceWatcher>
#include <QDateTime>
#include <QJsonArray>
#include <QJsonDocument>
#include <QSet>
#include <QTimer>
#include <QUuid>

#include <linux/input-event-codes.h>
#include <xkbcommon/xkbcommon.h>

#include <chrono>
#include <cstdlib>
#include <algorithm>

namespace
{

constexpr auto Service = "org.seatgeist.KWinBridge";
constexpr auto Path = "/org/seatgeist/KWinBridge1";
constexpr auto Interface = "org.seatgeist.KWinBridge1";
constexpr auto Backend = "kwin_agent_seat_v2";
constexpr auto BuildId = SEATGEIST_AGENT_SEAT_BUILD_MARKER;
constexpr int PollIntervalMs = 5000;
constexpr int DefaultRepeatRate = 25;
constexpr int DefaultRepeatDelayMs = 600;
constexpr int MaxAgentLanes = 4;
// Well below the daemon's three-second action deadline. This is an upper
// bound, not a blind startup delay: bound clients dispatch immediately.
constexpr int BindingTimeoutMs = 1000;
constexpr int BindingCheckIntervalMs = 10;

QDBusMessage methodCall(const QString &method)
{
    return QDBusMessage::createMethodCall(
        QString::fromLatin1(Service),
        QString::fromLatin1(Path),
        QString::fromLatin1(Interface),
        method);
}

} // namespace

namespace KWin
{

SeatgeistAgentSeatPlugin::SeatgeistAgentSeatPlugin()
{
    m_monotonicClock.start();
    m_bindings = std::make_unique<SeatBindings>(*waylandServer()->display());
    m_altClickTimer = new QTimer(this);
    m_altClickTimer->setInterval(10);
    connect(m_altClickTimer, &QTimer::timeout, this, [this] {
        if (m_altClick) { m_altClick->tick(m_monotonicClock.elapsed()); }
    });
    m_bindingTimer = new QTimer(this);
    m_bindingTimer->setInterval(BindingCheckIntervalMs);
    connect(m_bindingTimer, &QTimer::timeout, this, &SeatgeistAgentSeatPlugin::resumePendingAction);
    input()->installInputEventSpy(this);
    QString keymapError;
    if (!initializeKeymap(&keymapError)) {
        qWarning("Seatgeist agent seat keymap unavailable: %s", qPrintable(keymapError));
    } else {
        // GTK 3 processes deferred wl_seat bindings during display startup
        // only. Announce the permanent pool before applications connect, not
        // on their first action; retained seats then survive daemon restarts.
        for (int index = 0; index < MaxAgentLanes; ++index) {
            auto lane = std::make_unique<AgentLane>();
            QString error;
            if (!initializeLane(*lane, &error)) {
                qWarning("Seatgeist permanent seat initialization failed: %s", qPrintable(error));
                break;
            }
            m_idleLanes.push_back(std::move(lane));
        }
    }

    m_serviceWatcher = std::make_unique<QDBusServiceWatcher>(
        QString::fromLatin1(Service),
        QDBusConnection::sessionBus(),
        QDBusServiceWatcher::WatchForOwnerChange,
        this);
    connect(
        m_serviceWatcher.get(),
        &QDBusServiceWatcher::serviceOwnerChanged,
        this,
        [this](const QString &, const QString &, const QString &newOwner) {
            ++m_serviceGeneration;
            // Never deliver an action authorized by a departed daemon owner.
            detachDaemon();
            m_serviceAvailable = !newOwner.isEmpty();
            if (!m_serviceAvailable) {
                m_pollTimer->stop();
                return;
            }
            registerBackend();
            m_pollTimer->start();
        });

    m_pollTimer = new QTimer(this);
    m_pollTimer->setInterval(PollIntervalMs);
    connect(m_pollTimer, &QTimer::timeout, this, &SeatgeistAgentSeatPlugin::poll);
    auto *busInterface = QDBusConnection::sessionBus().interface();
    m_serviceAvailable = busInterface && busInterface->isServiceRegistered(
        QString::fromLatin1(Service));
    if (m_serviceAvailable) {
        registerBackend();
        m_pollTimer->start();
    }
}

SeatgeistAgentSeatPlugin::~SeatgeistAgentSeatPlugin()
{
    clearLanes();
    if (m_xkbKeymap) {
        xkb_keymap_unref(m_xkbKeymap);
    }
    if (m_xkbContext) {
        xkb_context_unref(m_xkbContext);
    }
}

bool SeatgeistAgentSeatPlugin::initializeKeymap(QString *error)
{
    m_xkbContext = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
    if (!m_xkbContext) {
        *error = QStringLiteral("xkb_context_new failed");
        return false;
    }
    m_xkbKeymap = xkb_keymap_new_from_names(
        m_xkbContext,
        nullptr,
        XKB_KEYMAP_COMPILE_NO_FLAGS);
    if (!m_xkbKeymap) {
        *error = QStringLiteral("xkb_keymap_new_from_names failed");
        return false;
    }
    return true;
}

void SeatgeistAgentSeatPlugin::registerBackend()
{
    if (!m_serviceAvailable) {
        return;
    }
    auto message = methodCall(QStringLiteral("RegisterAgentSeatBackendV3"));
    message << QString::fromLatin1(Backend);
    message << QString::fromLatin1(BuildId);
    message << QStringLiteral("relative_pointer_v1,alt_point_click_v1");
    auto *watcher = new QDBusPendingCallWatcher(QDBusConnection::sessionBus().asyncCall(message), this);
    connect(watcher, &QDBusPendingCallWatcher::finished, this, [this, watcher, generation = m_serviceGeneration] {
        QDBusPendingReply<> reply = *watcher;
        watcher->deleteLater();
        if (!reply.isError() && generation == m_serviceGeneration) {
            // The five-second watchdog exceeds the daemon action timeout.
            // Begin the first pull immediately after registration succeeds.
            poll();
        }
    });
}

void SeatgeistAgentSeatPlugin::poll()
{
    if (!m_serviceAvailable || m_pollInFlight || !m_pendingAction.isEmpty() || m_altClick) {
        return;
    }
    m_pollInFlight = true;
    auto message = methodCall(QStringLiteral("TakePendingAgentSeatAction"));
    auto *watcher = new QDBusPendingCallWatcher(
        QDBusConnection::sessionBus().asyncCall(message),
        this);
    watcher->setProperty("seatgeistServiceGeneration", QVariant::fromValue(m_serviceGeneration));
    connect(
        watcher,
        &QDBusPendingCallWatcher::finished,
        this,
        &SeatgeistAgentSeatPlugin::handlePendingAction);
}

void SeatgeistAgentSeatPlugin::handlePendingAction(QDBusPendingCallWatcher *watcher)
{
    QDBusPendingReply<QString> reply = *watcher;
    watcher->deleteLater();
    if (watcher->property("seatgeistServiceGeneration").toULongLong() != m_serviceGeneration) {
        QTimer::singleShot(0, this, &SeatgeistAgentSeatPlugin::poll);
        return;
    }
    m_pollInFlight = false;
    if (reply.isError()) {
        return;
    }
    if (!reply.value().isEmpty()) {
        QJsonParseError parseError;
        const auto document = QJsonDocument::fromJson(reply.value().toUtf8(), &parseError);
        if (parseError.error == QJsonParseError::NoError && document.isObject()) {
            executeAction(document.object());
        }
    }
    // The daemon keeps this call pending until work arrives or its heartbeat
    // timeout expires. Re-arm immediately after a successful response; the
    // periodic timer is only a watchdog for stalled/error calls.
    QTimer::singleShot(0, this, &SeatgeistAgentSeatPlugin::poll);
}

void SeatgeistAgentSeatPlugin::executeAction(const QJsonObject &action)
{
    const QString id = action.value(QStringLiteral("id")).toString();
    if (id.isEmpty()) {
        return;
    }
    QString error;
    Window *window = resolveNativeWaylandWindow(
        action.value(QStringLiteral("window_id")).toString(),
        &error);
    if (!window) {
        complete(id, false, error);
        return;
    }
    if (!decodeSeatEvents(action, window->clientGeometry().size(), &m_pendingEvents, &error)) {
        complete(id, false, error);
        return;
    }
    const auto roster = action.value(QStringLiteral("active_lane_ids"));
    QSet<QString> activeLanes;
    if (roster.isArray()) {
        for (const auto value : roster.toArray()) {
            const auto laneId = value.toString();
            if (QUuid(laneId).isNull() || activeLanes.contains(laneId)) {
                complete(id, false, QStringLiteral("invalid active lane roster"));
                return;
            }
            activeLanes.insert(laneId);
        }
    }
    if (activeLanes.size() > MaxAgentLanes
        || !activeLanes.contains(action.value(QStringLiteral("lane_id")).toString())) {
        complete(id, false, QStringLiteral("agent lane lease is inactive or exceeds quota"));
        return;
    }
    // Only released/expired owners can be reclaimed. Never evict a live
    // independent Codex just because its last action was less recent.
    for (auto it = m_lanes.begin(); it != m_lanes.end();) {
        if (!activeLanes.contains(it->first)) {
            clearLaneTarget(*it->second);
            m_idleLanes.push_back(std::move(it->second));
            it = m_lanes.erase(it);
        } else { ++it; }
    }
    AgentLane *lane = laneFor(action.value(QStringLiteral("lane_id")).toString(), &error);
    if (!lane) {
        complete(id, false, error);
        return;
    }

    if (!m_bindings->available()) {
        complete(id, false, QStringLiteral("agent-seat binding observation is unavailable"));
        return;
    }
    m_pendingAction = action;
    m_pendingWindow = window;
    m_pendingSurface = window->surface();
    m_pendingCancelled = false;
    m_bindingDeadlineMs = m_monotonicClock.elapsed() + BindingTimeoutMs;
    resumePendingAction();
}

void SeatgeistAgentSeatPlugin::resumePendingAction()
{
    if (m_pendingAction.isEmpty()) {
        return;
    }
    const QString laneId = m_pendingAction.value(QStringLiteral("lane_id")).toString();
    const auto laneIt = m_lanes.find(laneId);
    Window *window = m_pendingWindow;
    QString error;
    if (!m_serviceAvailable || m_pendingCancelled) {
        error = QStringLiteral("agent target received user input before delivery");
    } else if (!window || !m_pendingSurface || window->surface() != m_pendingSurface
               || laneIt == m_lanes.end()) {
        error = QStringLiteral("agent-seat target was lost before delivery");
    } else if (!validateClientFocusIsolation(window, &error)) {
        // A second window may have appeared during the binding wait.
    } else if (m_monotonicClock.elapsed() >= m_bindingDeadlineMs) {
        error = QStringLiteral("target did not bind the agent-seat input device before the deadline");
    } else if (m_pendingAction.contains(QStringLiteral("expires_at_unix_ms"))
               && m_pendingAction.value(QStringLiteral("expires_at_unix_ms")).toDouble()
                   <= QDateTime::currentMSecsSinceEpoch()) {
        error = QStringLiteral("agent-seat action expired before delivery");
    } else {
        auto *seat = laneIt->second->seat.get();
        auto *client = window->surface()->client()->client();
        const bool keyboard = std::any_of(m_pendingEvents.begin(), m_pendingEvents.end(),
            [](const auto &event) { return event.kind == SeatEvent::Key || event.kind == SeatEvent::AltPointClick; });
        const bool pointer = std::any_of(m_pendingEvents.begin(), m_pendingEvents.end(),
            [](const auto &event) { return event.kind != SeatEvent::Key; });
        const bool relative = std::any_of(m_pendingEvents.begin(), m_pendingEvents.end(),
            [](const auto &event) { return event.kind == SeatEvent::RelativeMotion; });
        // Relative delivery never enters, warps, or focuses a new target. The
        // client must already have acquired its lock on this exact agent seat.
        if (relative && (laneIt->second->target != window
            || !m_bindings->lockedPointerReady(seat, client, window->surface()))) {
            complete(m_pendingAction.value("id").toString(), false, QStringLiteral("relative pointer requires an active lock on the same agent seat/client/target"));
            m_bindingTimer->stop();
            m_pendingAction = {}; m_pendingWindow = nullptr; m_pendingSurface = nullptr; m_pendingEvents.clear();
            QTimer::singleShot(0, this, &SeatgeistAgentSeatPlugin::poll);
            return;
        }
        const bool ready = (!relative || m_bindings->relativePointerReady(seat, client)) && (!keyboard || m_bindings->keyboardReady(seat, client))
            && (!pointer || m_bindings->pointerReady(seat, client));
        if (!ready) {
            // Return to KWin's event loop so clients can process the new
            // global, bind the seat, and create their keyboard/pointer.
            m_bindingTimer->start();
            return;
        }
    }
    m_bindingTimer->stop();
    const QJsonObject action = m_pendingAction;
    m_pendingAction = {};
    m_pendingWindow = nullptr;
    m_pendingSurface = nullptr;
    m_pendingEvents.clear();
    if (!error.isEmpty()) {
        complete(action.value(QStringLiteral("id")).toString(), false, error);
    } else {
        dispatchAction(action, *laneIt->second, window);
    }
    QTimer::singleShot(0, this, &SeatgeistAgentSeatPlugin::poll);
}

void SeatgeistAgentSeatPlugin::dispatchAction(const QJsonObject &action, AgentLane &lane, Window *window)
{
    QString error;
    QVector<SeatEvent> events;
    // Recheck all coordinates after the asynchronous binding wait, before any
    // event. Resizing the target must not turn a valid drag into partial input.
    if (!decodeSeatEvents(action, window->clientGeometry().size(), &events, &error)) {
        complete(action.value("id").toString(), false, error);
        return;
    }
    if (events.first().kind == SeatEvent::AltPointClick) {
        startAltPointClick(action, lane, window, events.first().position);
        return;
    }
    if (events.first().kind == SeatEvent::RelativeMotion) {
        auto *client = window->surface()->client()->client();
        const auto timestamp = std::chrono::microseconds(m_monotonicClock.nsecsElapsed() / 1000);
        if (lane.target != window || !m_bindings->deliverRelativePointer(lane.seat.get(), client, window->surface(), events.first().position, timestamp)) {
            complete(action.value("id").toString(), false, QStringLiteral("relative pointer binding or active lock was lost before delivery"));
            return;
        }
        // Successful delivery is one relative protocol event, without a warp
        // or a change to the physical seat. It does not prove app consumption.
        complete(action.value("id").toString(), true);
        return;
    }
    QPointF position = lane.target == window ? lane.localPointerPosition
        : QPointF(window->clientGeometry().width() / 2.0, window->clientGeometry().height() / 2.0);
    if (events.first().kind == SeatEvent::Motion) {
        position = events.first().position;
    }
    if (!bindTarget(lane, window, position, &error)) {
        complete(action.value("id").toString(), false, error);
        return;
    }
    for (const auto &event : events) {
        setTimestamp(lane);
        switch (event.kind) {
        case SeatEvent::Key:
            lane.seat->notifyKeyboardKey(event.code,
                event.pressed ? KeyboardKeyState::Pressed : KeyboardKeyState::Released,
                waylandServer()->display()->nextSerial());
            xkb_state_update_key(lane.xkbState, event.code + 8, event.pressed ? XKB_KEY_DOWN : XKB_KEY_UP);
            updateKeyboardModifiers(lane);
            break;
        case SeatEvent::AltPointClick:
        case SeatEvent::RelativeMotion:
            Q_UNREACHABLE(); // decoder requires a standalone relative event
        case SeatEvent::Motion:
            // The whole batch was checked against the current surface above;
            // this synchronous loop never processes client/target changes.
            bindTarget(lane, window, event.position, &error);
            break;
        case SeatEvent::Button:
            lane.seat->notifyPointerButton(event.code,
                event.pressed ? PointerButtonState::Pressed : PointerButtonState::Released);
            lane.seat->notifyPointerFrame();
            break;
        case SeatEvent::Axis:
            lane.seat->notifyPointerAxis(event.axis, event.delta, event.deltaV120, PointerAxisSource::Wheel);
            lane.seat->notifyPointerFrame();
            break;
        }
    }
    complete(action.value("id").toString(), true);
}

void SeatgeistAgentSeatPlugin::startAltPointClick(const QJsonObject &action, AgentLane &lane, Window *window, const QPointF &point)
{
    const QString id = action.value("id").toString();
    const QString laneId = action.value("lane_id").toString();
    const QString sessionId = action.value("session_id").toString();
    const QString windowId = action.value("window_id").toString();
    if (m_altClick || sessionId.isEmpty() || lane.target != window) {
        complete(id, false, QStringLiteral("Alt-point click needs its existing exact lane/session target")); return;
    }
    const QPointer<Window> target = window;
    const QPointer<SurfaceInterface> surface = window->surface();
    const auto app = window->resourceClass(); const auto desktop = window->desktopFileName(); const auto pid = window->pid();
    const auto geometry=window->clientGeometry();
    const auto transform=window->inputTransformation();
    const auto policyApp = desktop.isEmpty() ? app : desktop;
    if (action.value("expected_app_id").toString() != policyApp || action.value("expected_pid").toDouble() != pid || pid <= 0) {
        complete(id, false, QStringLiteral("Alt-point click pinned app/PID changed before delivery")); return;
    }
    const auto generation = m_serviceGeneration;
    const auto expires = action.value("expires_at_unix_ms").toDouble();
    m_altClickId = id; m_altClickWindow = window;
    AltPointClick::Hooks hooks;
    hooks.valid = [this, target, surface, app, desktop, pid, geometry, transform, laneId, generation, expires, point] {
        auto it = m_lanes.find(laneId); QString error;
        return m_serviceAvailable && generation == m_serviceGeneration && target && surface
            && it != m_lanes.end() && it->second->target == target && target->surface() == surface
            && target->resourceClass() == app && target->desktopFileName() == desktop && target->pid() == pid
            && target->clientGeometry() == geometry && target->inputTransformation() == transform
            && expires > QDateTime::currentMSecsSinceEpoch()
            && QRectF(QPointF(0,0), target->clientGeometry().size()).contains(point)
            && validateClientFocusIsolation(target, &error);
    };
    hooks.alt = [this, laneId, surface](bool pressed) {
        auto it = m_lanes.find(laneId); if (it == m_lanes.end()) { return; }
        auto &lane = *it->second; setTimestamp(lane);
        // Cleanup resets the virtual modifier even if the original surface is
        // gone; never send a key event into a replacement keyboard target.
        if (surface && lane.seat->focusedKeyboardSurface() == surface) {
            lane.seat->notifyKeyboardKey(KEY_LEFTALT, pressed ? KeyboardKeyState::Pressed : KeyboardKeyState::Released,
                                        waylandServer()->display()->nextSerial());
        }
        xkb_state_update_key(lane.xkbState, KEY_LEFTALT + 8, pressed ? XKB_KEY_DOWN : XKB_KEY_UP);
        updateKeyboardModifiers(lane);
    };
    hooks.authorize = [this, id, laneId, sessionId, generation, windowId] {
        auto message = methodCall(QStringLiteral("CheckAltPointClickAction"));
        message << id << sessionId << laneId << windowId;
        auto *watcher = new QDBusPendingCallWatcher(QDBusConnection::sessionBus().asyncCall(message), this);
        connect(watcher, &QDBusPendingCallWatcher::finished, this, [this, watcher, id, generation] {
            QDBusPendingReply<bool> reply = *watcher; watcher->deleteLater();
            if (m_altClick && m_altClickId == id && generation == m_serviceGeneration) {
                m_altClick->authorized(!reply.isError() && reply.value(), m_monotonicClock.elapsed());
            }
        });
    };
    hooks.click = [this, laneId, target, point] {
        auto it = m_lanes.find(laneId); QString error;
        if (!target || it == m_lanes.end() || !bindTarget(*it->second, target, point, &error)) { return false; }
        auto &lane = *it->second; setTimestamp(lane);
        lane.seat->notifyPointerButton(BTN_LEFT, PointerButtonState::Pressed);
        lane.seat->notifyPointerButton(BTN_LEFT, PointerButtonState::Released);
        lane.seat->notifyPointerFrame(); return true;
    };
    hooks.finished = [this, id, generation](bool ok, const QString &error) {
        m_altClickTimer->stop();
        if (generation == m_serviceGeneration && m_serviceAvailable) { complete(id, ok, error); }
        QTimer::singleShot(0, this, [this, id] {
            if (m_altClickId == id) { m_altClick.reset(); m_altClickId.clear(); m_altClickWindow = nullptr; }
            poll();
        });
    };
    m_altClick = std::make_unique<AltPointClick>(*m_bindings, lane.seat.get(), surface, std::move(hooks));
    m_altClickTimer->start();
    m_altClick->start(m_monotonicClock.elapsed());
}

void SeatgeistAgentSeatPlugin::cancelPendingFor(Window *window)
{
    if (window && window == m_altClickWindow && m_altClick) {
        m_altClick->cancel(QStringLiteral("Alt-point click received physical user input"));
    }
    if (window && window == m_pendingWindow) {
        m_pendingCancelled = true;
    }
}

void SeatgeistAgentSeatPlugin::keyboardKey(KeyboardKeyEvent *)
{
    cancelPendingFor(workspace()->activeWindow());
}

void SeatgeistAgentSeatPlugin::pointerMotion(PointerMotionEvent *event)
{
    cancelPendingFor(input()->findToplevel(event->position));
}

void SeatgeistAgentSeatPlugin::pointerButton(PointerButtonEvent *event)
{
    cancelPendingFor(input()->findToplevel(event->position));
}

void SeatgeistAgentSeatPlugin::pointerAxis(PointerAxisEvent *event)
{
    cancelPendingFor(input()->findToplevel(event->position));
}

void SeatgeistAgentSeatPlugin::touchDown(TouchDownEvent *event)
{
    auto *window = input()->findToplevel(event->pos);
    m_touchTargets[event->id] = window;
    cancelPendingFor(window);
}

void SeatgeistAgentSeatPlugin::touchMotion(TouchMotionEvent *event)
{
    const auto found = m_touchTargets.find(event->id);
    if (found != m_touchTargets.end()) {
        cancelPendingFor(found->second);
    }
    cancelPendingFor(input()->findToplevel(event->pos));
}

void SeatgeistAgentSeatPlugin::touchUp(TouchUpEvent *event)
{
    const auto found = m_touchTargets.find(event->id);
    if (found != m_touchTargets.end()) {
        cancelPendingFor(found->second);
        m_touchTargets.erase(found);
    }
}

Window *SeatgeistAgentSeatPlugin::resolveNativeWaylandWindow(
    const QString &windowId,
    QString *error) const
{
    const QUuid uuid(windowId);
    if (uuid.isNull()) {
        *error = QStringLiteral("invalid KWin window id");
        return nullptr;
    }
    Window *window = workspace()->findWindow(uuid);
    if (!window) {
        *error = QStringLiteral("target window is no longer available");
        return nullptr;
    }
    if (window->isInternal() || !window->surface()) {
        *error = QStringLiteral("target is not a native Wayland client window");
        return nullptr;
    }
    if (!validateClientFocusIsolation(window, error)) {
        return nullptr;
    }
    return window;
}

bool SeatgeistAgentSeatPlugin::validateClientFocusIsolation(Window *window, QString *error) const
{
    // Firefox's GTK key callbacks route through process-global gFocusWindow,
    // not the event's seat/window. Real two-window tests receive another
    // owner's keys despite correct wl_keyboard delivery. Do not silently
    // share that client; AT-SPI or a dedicated browser process is required.
    if (!window->resourceClass().contains(QStringLiteral("firefox"), Qt::CaseInsensitive)) {
        return true;
    }
    for (auto *other : workspace()->windows()) {
        if (other != window && !other->isInternal() && other->surface()
            && (other->isNormalWindow() || other->isDialog())
            && other->surface()->client() == window->surface()->client()) {
            *error = QStringLiteral("Firefox shared-process window focus is not seat-isolated; use a dedicated Firefox process/profile or exact-window semantic actions");
            return false;
        }
    }
    return true;
}

SeatgeistAgentSeatPlugin::AgentLane *SeatgeistAgentSeatPlugin::laneFor(
    const QString &laneId,
    QString *error)
{
    const QUuid uuid(laneId);
    if (uuid.isNull()) {
        *error = QStringLiteral("invalid agent lane id");
        return nullptr;
    }
    auto existing = m_lanes.find(laneId);
    if (existing != m_lanes.end()) {
        return existing->second.get();
    }
    if (!m_idleLanes.empty()) {
        auto lane = std::move(m_idleLanes.back());
        m_idleLanes.pop_back();
        auto *result = lane.get();
        m_lanes.emplace(laneId, std::move(lane));
        return result;
    }
    // Do not silently introduce late globals on partial initialization either.
    *error = QStringLiteral("no idle permanent agent seat is available");
    return nullptr;
}

bool SeatgeistAgentSeatPlugin::initializeLane(AgentLane &lane, QString *error)
{
    if (!m_xkbKeymap) {
        *error = QStringLiteral("agent-seat keymap is unavailable");
        return false;
    }
    lane.seat = std::make_unique<SeatInterface>(
        waylandServer()->display(),
        QStringLiteral("seatgeist-agent-%1").arg(m_nextLaneIndex++),
        this);
    lane.seat->setHasPointer(true);
    lane.seat->setHasKeyboard(true);
    if (auto *primarySeat = waylandServer()->seat()) {
        auto *primaryKeyboard = primarySeat->keyboard();
        lane.seat->keyboard()->setRepeatInfo(
            primaryKeyboard->keyRepeatRate(),
            primaryKeyboard->keyRepeatDelay());
    } else {
        qWarning("Seatgeist agent seat could not read the primary seat repeat settings; using defaults");
        lane.seat->keyboard()->setRepeatInfo(DefaultRepeatRate, DefaultRepeatDelayMs);
    }
    char *keymap = xkb_keymap_get_as_string(m_xkbKeymap, XKB_KEYMAP_FORMAT_TEXT_V1);
    if (!keymap) {
        *error = QStringLiteral("xkb_keymap_get_as_string failed");
        return false;
    }
    lane.seat->keyboard()->setKeymap(QByteArray(keymap));
    free(keymap);
    lane.xkbState = xkb_state_new(m_xkbKeymap);
    if (!lane.xkbState) {
        *error = QStringLiteral("xkb_state_new failed");
        return false;
    }
    m_bindings->watch(lane.seat.get());
    return true;
}

bool SeatgeistAgentSeatPlugin::bindTarget(
    AgentLane &lane,
    Window *window,
    const QPointF &localPosition,
    QString *error)
{
    if (!window || !window->surface()) {
        *error = QStringLiteral("target surface is unavailable");
        return false;
    }
    // ScreenShot2 exact-window captures exclude server-side decorations, so
    // Seatgeist's window-local coordinates are relative to the client surface,
    // not to frameGeometry(). inputTransformation() still expects the global
    // logical point and applies the compositor's output scaling internally.
    const QRectF client = window->clientGeometry();
    if (!QRectF(QPointF(0, 0), client.size()).contains(localPosition)) {
        *error = QStringLiteral("window-local pointer coordinate is outside the target");
        return false;
    }
    const QPointF globalPosition = client.topLeft() + localPosition;
    setTimestamp(lane);
    lane.seat->setFocusedKeyboardSurface(window->surface());
    if (lane.seat->focusedPointerSurface() == window->surface()) {
        // KWin suppresses enter events for an already-focused surface. Update
        // its transform and send motion so clicks do not use the old position.
        lane.seat->setFocusedPointerSurfaceTransformation(window->inputTransformation());
        lane.seat->notifyPointerMotion(globalPosition);
    } else {
        lane.seat->notifyPointerEnter(
            window->surface(),
            globalPosition,
            window->inputTransformation());
    }
    lane.seat->notifyPointerFrame();
    lane.target = window;
    lane.localPointerPosition = localPosition;
    return true;
}


void SeatgeistAgentSeatPlugin::updateKeyboardModifiers(AgentLane &lane)
{
    lane.seat->notifyKeyboardModifiers(
        xkb_state_serialize_mods(lane.xkbState, XKB_STATE_MODS_DEPRESSED),
        xkb_state_serialize_mods(lane.xkbState, XKB_STATE_MODS_LATCHED),
        xkb_state_serialize_mods(lane.xkbState, XKB_STATE_MODS_LOCKED),
        xkb_state_serialize_layout(lane.xkbState, XKB_STATE_LAYOUT_EFFECTIVE));
}

void SeatgeistAgentSeatPlugin::setTimestamp(AgentLane &lane)
{
    lane.seat->setTimestamp(std::chrono::microseconds(m_monotonicClock.nsecsElapsed() / 1000));
}

void SeatgeistAgentSeatPlugin::complete(
    const QString &id,
    bool ok,
    const QString &error)
{
    QJsonObject result{
        {QStringLiteral("id"), id},
        {QStringLiteral("ok"), ok},
        {QStringLiteral("backend"), QString::fromLatin1(Backend)},
    };
    if (!error.isEmpty()) {
        result.insert(QStringLiteral("error"), error);
    }
    auto message = methodCall(QStringLiteral("CompleteAgentSeatAction"));
    message << QString::fromUtf8(
        QJsonDocument(result).toJson(QJsonDocument::Compact));
    QDBusConnection::sessionBus().asyncCall(message);
}

void SeatgeistAgentSeatPlugin::clearLaneTarget(AgentLane &lane)
{
    if (!lane.seat) {
        return;
    }
    lane.seat->setFocusedKeyboardSurface(nullptr);
    lane.seat->notifyPointerLeave();
    lane.seat->notifyPointerFrame();
    lane.target = nullptr;
    lane.localPointerPosition = {};
    if (lane.xkbState) {
        // Balanced batches release depressed keys, but lock/latch state (for
        // example Caps Lock) must not leak to the next owner of this seat.
        xkb_state_update_mask(lane.xkbState, 0, 0, 0, 0, 0, 0);
        updateKeyboardModifiers(lane);
    }
}

void SeatgeistAgentSeatPlugin::detachDaemon()
{
    if (m_altClick) { m_altClick->cancel(QStringLiteral("Alt-point click daemon disconnected")); }
    m_bindingTimer->stop();
    m_pendingAction = {};
    m_pendingEvents.clear();
    m_pendingWindow = nullptr;
    m_pendingSurface = nullptr;
    m_pollInFlight = false;
    for (auto &[laneId, lane] : m_lanes) {
        Q_UNUSED(laneId)
        clearLaneTarget(*lane);
        m_idleLanes.push_back(std::move(lane));
    }
    m_lanes.clear();
}

void SeatgeistAgentSeatPlugin::clearLanes()
{
    detachDaemon();
    for (auto &lane : m_idleLanes) {
        m_bindings->forget(lane->seat.get());
        if (lane->xkbState) {
            xkb_state_unref(lane->xkbState);
            lane->xkbState = nullptr;
        }
    }
    m_idleLanes.clear();
}

} // namespace KWin

#include "moc_agentseatplugin.cpp"
