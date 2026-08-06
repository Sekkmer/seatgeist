// SPDX-License-Identifier: MIT OR Apache-2.0

#include "agentseatplugin.h"

#include <wayland/display.h>
#include <wayland/keyboard.h>
#include <wayland/seat.h>
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
#include <QJsonArray>
#include <QJsonDocument>
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
constexpr auto Backend = "kwin_agent_seat_v1";
constexpr int PollIntervalMs = 5000;
constexpr int DefaultRepeatRate = 25;
constexpr int DefaultRepeatDelayMs = 600;
constexpr int MaxAgentLanes = 4;

QDBusMessage methodCall(const QString &method)
{
    return QDBusMessage::createMethodCall(
        QString::fromLatin1(Service),
        QString::fromLatin1(Path),
        QString::fromLatin1(Interface),
        method);
}

bool finiteNumber(const QJsonValue &value)
{
    return value.isDouble() && qIsFinite(value.toDouble());
}

} // namespace

namespace KWin
{

SeatgeistAgentSeatPlugin::SeatgeistAgentSeatPlugin()
{
    m_monotonicClock.start();
    QString keymapError;
    if (!initializeKeymap(&keymapError)) {
        qWarning("Seatgeist agent seat keymap unavailable: %s", qPrintable(keymapError));
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
            m_serviceAvailable = !newOwner.isEmpty();
            if (!m_serviceAvailable) {
                m_pollTimer->stop();
                for (auto &[laneId, lane] : m_lanes) {
                    Q_UNUSED(laneId)
                    clearLaneTarget(*lane);
                }
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
    auto message = methodCall(QStringLiteral("RegisterAgentSeatBackend"));
    message << QString::fromLatin1(Backend);
    QDBusConnection::sessionBus().asyncCall(message);
}

void SeatgeistAgentSeatPlugin::poll()
{
    if (!m_serviceAvailable || m_pollInFlight) {
        return;
    }
    m_pollInFlight = true;
    auto message = methodCall(QStringLiteral("TakePendingAgentSeatAction"));
    auto *watcher = new QDBusPendingCallWatcher(
        QDBusConnection::sessionBus().asyncCall(message),
        this);
    connect(
        watcher,
        &QDBusPendingCallWatcher::finished,
        this,
        &SeatgeistAgentSeatPlugin::handlePendingAction);
}

void SeatgeistAgentSeatPlugin::handlePendingAction(QDBusPendingCallWatcher *watcher)
{
    m_pollInFlight = false;
    QDBusPendingReply<QString> reply = *watcher;
    watcher->deleteLater();
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
    AgentLane *lane = laneFor(action.value(QStringLiteral("lane_id")).toString(), &error);
    if (!lane) {
        complete(id, false, error);
        return;
    }

    const QString kind = action.value(QStringLiteral("action")).toString();
    bool ok = false;
    if (kind == QLatin1String("key_combo")) {
        ok = sendKeyCombo(action, *lane, window, &error);
    } else if (kind == QLatin1String("key_sequence")) {
        ok = sendKeySequence(action, *lane, window, &error);
    } else if (kind == QLatin1String("pointer_move")) {
        ok = movePointer(action, *lane, window, &error);
    } else if (kind == QLatin1String("pointer_click")) {
        ok = clickPointer(action, *lane, window, &error);
    } else if (kind == QLatin1String("pointer_drag")) {
        ok = dragPointer(action, *lane, window, &error);
    } else if (kind == QLatin1String("pointer_scroll")) {
        ok = scrollPointer(action, *lane, window, &error);
    } else {
        error = QStringLiteral("unsupported agent-seat action");
    }
    complete(id, ok, error);
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
    return window;
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
        existing->second->lastUsedMs = m_monotonicClock.elapsed();
        return existing->second.get();
    }
    if (m_lanes.size() >= MaxAgentLanes) {
        auto oldest = std::min_element(
            m_lanes.begin(),
            m_lanes.end(),
            [](const auto &left, const auto &right) {
                return left.second->lastUsedMs < right.second->lastUsedMs;
            });
        clearLaneTarget(*oldest->second);
        if (oldest->second->xkbState) {
            xkb_state_unref(oldest->second->xkbState);
        }
        m_lanes.erase(oldest);
    }
    auto lane = std::make_unique<AgentLane>();
    if (!initializeLane(*lane, error)) {
        return nullptr;
    }
    lane->lastUsedMs = m_monotonicClock.elapsed();
    AgentLane *result = lane.get();
    m_lanes.emplace(laneId, std::move(lane));
    return result;
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
    lane.seat->notifyPointerEnter(
        window->surface(),
        globalPosition,
        window->inputTransformation());
    lane.seat->notifyPointerFrame();
    lane.target = window;
    lane.localPointerPosition = localPosition;
    return true;
}

bool SeatgeistAgentSeatPlugin::sendKeyCombo(
    const QJsonObject &action,
    AgentLane &lane,
    Window *window,
    QString *error)
{
    if (!lane.xkbState) {
        *error = QStringLiteral("agent-seat keymap is unavailable");
        return false;
    }
    const QJsonArray keycodes = action.value(QStringLiteral("keycodes")).toArray();
    if (keycodes.isEmpty() || keycodes.size() > 8) {
        *error = QStringLiteral("key combo must contain between 1 and 8 keys");
        return false;
    }
    const QPointF position = lane.target == window
        ? lane.localPointerPosition
        : QPointF(
            window->clientGeometry().width() / 2.0,
            window->clientGeometry().height() / 2.0);
    if (!bindTarget(lane, window, position, error)) {
        return false;
    }

    QList<quint32> codes;
    for (const QJsonValue &value : keycodes) {
        const int code = value.toInt(-1);
        if (code <= 0 || code > KEY_MAX) {
            *error = QStringLiteral("key combo contains an invalid evdev keycode");
            return false;
        }
        codes.push_back(static_cast<quint32>(code));
    }
    for (const quint32 code : codes) {
        setTimestamp(lane);
        lane.seat->notifyKeyboardKey(
            code,
            KeyboardKeyState::Pressed,
            waylandServer()->display()->nextSerial());
        xkb_state_update_key(lane.xkbState, code + 8, XKB_KEY_DOWN);
        updateKeyboardModifiers(lane);
    }
    for (auto it = codes.crbegin(); it != codes.crend(); ++it) {
        const quint32 code = *it;
        setTimestamp(lane);
        lane.seat->notifyKeyboardKey(
            code,
            KeyboardKeyState::Released,
            waylandServer()->display()->nextSerial());
        xkb_state_update_key(lane.xkbState, code + 8, XKB_KEY_UP);
        updateKeyboardModifiers(lane);
    }
    return true;
}

bool SeatgeistAgentSeatPlugin::sendKeySequence(
    const QJsonObject &action,
    AgentLane &lane,
    Window *window,
    QString *error)
{
    if (!lane.xkbState) {
        *error = QStringLiteral("agent-seat keymap is unavailable");
        return false;
    }
    const QJsonArray chords = action.value(QStringLiteral("chords")).toArray();
    if (chords.isEmpty() || chords.size() > 8192) {
        *error = QStringLiteral("key sequence must contain between 1 and 8192 chords");
        return false;
    }
    const QPointF position = lane.target == window
        ? lane.localPointerPosition
        : QPointF(
            window->clientGeometry().width() / 2.0,
            window->clientGeometry().height() / 2.0);
    if (!bindTarget(lane, window, position, error)) {
        return false;
    }

    for (const QJsonValue &chordValue : chords) {
        const QJsonArray chord = chordValue.toArray();
        if (chord.isEmpty() || chord.size() > 2) {
            *error = QStringLiteral("key sequence contains an invalid chord");
            return false;
        }
        QList<quint32> codes;
        for (const QJsonValue &value : chord) {
            const int code = value.toInt(-1);
            if (code <= 0 || code > KEY_MAX) {
                *error = QStringLiteral("key sequence contains an invalid evdev keycode");
                return false;
            }
            codes.push_back(static_cast<quint32>(code));
        }
        for (const quint32 code : codes) {
            setTimestamp(lane);
            lane.seat->notifyKeyboardKey(
                code,
                KeyboardKeyState::Pressed,
                waylandServer()->display()->nextSerial());
            xkb_state_update_key(lane.xkbState, code + 8, XKB_KEY_DOWN);
            updateKeyboardModifiers(lane);
        }
        for (auto it = codes.crbegin(); it != codes.crend(); ++it) {
            const quint32 code = *it;
            setTimestamp(lane);
            lane.seat->notifyKeyboardKey(
                code,
                KeyboardKeyState::Released,
                waylandServer()->display()->nextSerial());
            xkb_state_update_key(lane.xkbState, code + 8, XKB_KEY_UP);
            updateKeyboardModifiers(lane);
        }
    }
    return true;
}

bool SeatgeistAgentSeatPlugin::movePointer(
    const QJsonObject &action,
    AgentLane &lane,
    Window *window,
    QString *error)
{
    if (!finiteNumber(action.value(QStringLiteral("x")))
        || !finiteNumber(action.value(QStringLiteral("y")))) {
        *error = QStringLiteral("pointer coordinates must be finite numbers");
        return false;
    }
    return bindTarget(
        lane,
        window,
        QPointF(
            action.value(QStringLiteral("x")).toDouble(),
            action.value(QStringLiteral("y")).toDouble()),
        error);
}

bool SeatgeistAgentSeatPlugin::clickPointer(
    const QJsonObject &action,
    AgentLane &lane,
    Window *window,
    QString *error)
{
    if (!movePointer(action, lane, window, error)) {
        return false;
    }
    const int button = action.value(QStringLiteral("button")).toInt(-1);
    const int clicks = action.value(QStringLiteral("clicks")).toInt(0);
    if ((button != BTN_LEFT && button != BTN_MIDDLE && button != BTN_RIGHT)
        || clicks < 1 || clicks > 2) {
        *error = QStringLiteral("invalid pointer button or click count");
        return false;
    }
    for (int click = 0; click < clicks; ++click) {
        setTimestamp(lane);
        lane.seat->notifyPointerButton(
            static_cast<quint32>(button),
            PointerButtonState::Pressed);
        lane.seat->notifyPointerFrame();
        setTimestamp(lane);
        lane.seat->notifyPointerButton(
            static_cast<quint32>(button),
            PointerButtonState::Released);
        lane.seat->notifyPointerFrame();
    }
    return true;
}

bool SeatgeistAgentSeatPlugin::dragPointer(
    const QJsonObject &action,
    AgentLane &lane,
    Window *window,
    QString *error)
{
    const QJsonValue fromX = action.value(QStringLiteral("from_x"));
    const QJsonValue fromY = action.value(QStringLiteral("from_y"));
    const QJsonValue toX = action.value(QStringLiteral("to_x"));
    const QJsonValue toY = action.value(QStringLiteral("to_y"));
    const int button = action.value(QStringLiteral("button")).toInt(-1);
    if (!finiteNumber(fromX) || !finiteNumber(fromY)
        || !finiteNumber(toX) || !finiteNumber(toY)
        || (button != BTN_LEFT && button != BTN_MIDDLE && button != BTN_RIGHT)) {
        *error = QStringLiteral("invalid pointer drag");
        return false;
    }
    if (!bindTarget(lane, window, QPointF(fromX.toDouble(), fromY.toDouble()), error)) {
        return false;
    }
    setTimestamp(lane);
    lane.seat->notifyPointerButton(
        static_cast<quint32>(button),
        PointerButtonState::Pressed);
    lane.seat->notifyPointerFrame();
    if (!bindTarget(lane, window, QPointF(toX.toDouble(), toY.toDouble()), error)) {
        lane.seat->notifyPointerButton(
            static_cast<quint32>(button),
            PointerButtonState::Released);
        lane.seat->notifyPointerFrame();
        return false;
    }
    setTimestamp(lane);
    lane.seat->notifyPointerButton(
        static_cast<quint32>(button),
        PointerButtonState::Released);
    lane.seat->notifyPointerFrame();
    return true;
}

bool SeatgeistAgentSeatPlugin::scrollPointer(
    const QJsonObject &action,
    AgentLane &lane,
    Window *window,
    QString *error)
{
    const int vertical = action.value(QStringLiteral("vertical")).toInt();
    const int horizontal = action.value(QStringLiteral("horizontal")).toInt();
    if (vertical == 0 && horizontal == 0) {
        *error = QStringLiteral("scroll delta must be non-zero");
        return false;
    }
    const QPointF position = lane.target == window
        ? lane.localPointerPosition
        : QPointF(
            window->clientGeometry().width() / 2.0,
            window->clientGeometry().height() / 2.0);
    if (!bindTarget(lane, window, position, error)) {
        return false;
    }
    setTimestamp(lane);
    if (vertical != 0) {
        lane.seat->notifyPointerAxis(
            Qt::Vertical,
            vertical * 15.0,
            vertical * 120,
            PointerAxisSource::Wheel);
    }
    if (horizontal != 0) {
        lane.seat->notifyPointerAxis(
            Qt::Horizontal,
            horizontal * 15.0,
            horizontal * 120,
            PointerAxisSource::Wheel);
    }
    lane.seat->notifyPointerFrame();
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
}

void SeatgeistAgentSeatPlugin::clearLanes()
{
    for (auto &[laneId, lane] : m_lanes) {
        Q_UNUSED(laneId)
        clearLaneTarget(*lane);
        if (lane->xkbState) {
            xkb_state_unref(lane->xkbState);
            lane->xkbState = nullptr;
        }
    }
    m_lanes.clear();
}

} // namespace KWin

#include "moc_agentseatplugin.cpp"
