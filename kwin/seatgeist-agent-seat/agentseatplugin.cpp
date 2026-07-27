// SPDX-License-Identifier: MIT OR Apache-2.0

#include "agentseatplugin.h"

#include <wayland/display.h>
#include <wayland/keyboard.h>
#include <wayland/seat.h>
#include <wayland_server.h>
#include <window.h>
#include <workspace.h>

#include <QDBusConnection>
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

namespace
{

constexpr auto Service = "org.seatgeist.KWinBridge";
constexpr auto Path = "/org/seatgeist/KWinBridge1";
constexpr auto Interface = "org.seatgeist.KWinBridge1";
constexpr auto Backend = "kwin_agent_seat_v1";
constexpr int PollIntervalMs = 10;
constexpr int DefaultRepeatRate = 25;
constexpr int DefaultRepeatDelayMs = 600;

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
    m_seat = std::make_unique<SeatInterface>(
        waylandServer()->display(),
        QStringLiteral("seatgeist-agent-0"),
        this);
    m_seat->setHasPointer(true);
    m_seat->setHasKeyboard(true);

    if (auto *primarySeat = waylandServer()->seat()) {
        auto *primaryKeyboard = primarySeat->keyboard();
        m_seat->keyboard()->setRepeatInfo(
            primaryKeyboard->keyRepeatRate(),
            primaryKeyboard->keyRepeatDelay());
    } else {
        qWarning("Seatgeist agent seat could not read the primary seat repeat settings; using defaults");
        m_seat->keyboard()->setRepeatInfo(DefaultRepeatRate, DefaultRepeatDelayMs);
    }

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
            if (newOwner.isEmpty()) {
                clearTarget();
                return;
            }
            registerBackend();
        });

    m_pollTimer = new QTimer(this);
    m_pollTimer->setInterval(PollIntervalMs);
    connect(m_pollTimer, &QTimer::timeout, this, &SeatgeistAgentSeatPlugin::poll);
    m_pollTimer->start();
    registerBackend();
}

SeatgeistAgentSeatPlugin::~SeatgeistAgentSeatPlugin()
{
    clearTarget();
    if (m_xkbState) {
        xkb_state_unref(m_xkbState);
    }
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
    m_xkbState = xkb_state_new(m_xkbKeymap);
    if (!m_xkbState) {
        *error = QStringLiteral("xkb_state_new failed");
        return false;
    }
    char *keymap = xkb_keymap_get_as_string(m_xkbKeymap, XKB_KEYMAP_FORMAT_TEXT_V1);
    if (!keymap) {
        *error = QStringLiteral("xkb_keymap_get_as_string failed");
        return false;
    }
    m_seat->keyboard()->setKeymap(QByteArray(keymap));
    free(keymap);
    return true;
}

void SeatgeistAgentSeatPlugin::registerBackend()
{
    auto message = methodCall(QStringLiteral("RegisterAgentSeatBackend"));
    message << QString::fromLatin1(Backend);
    QDBusConnection::sessionBus().asyncCall(message);
}

void SeatgeistAgentSeatPlugin::poll()
{
    if (m_pollInFlight) {
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
    if (reply.isError() || reply.value().isEmpty()) {
        return;
    }
    QJsonParseError parseError;
    const auto document = QJsonDocument::fromJson(reply.value().toUtf8(), &parseError);
    if (parseError.error != QJsonParseError::NoError || !document.isObject()) {
        return;
    }
    executeAction(document.object());
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

    const QString kind = action.value(QStringLiteral("action")).toString();
    bool ok = false;
    if (kind == QLatin1String("key_combo")) {
        ok = sendKeyCombo(action, window, &error);
    } else if (kind == QLatin1String("key_sequence")) {
        ok = sendKeySequence(action, window, &error);
    } else if (kind == QLatin1String("pointer_move")) {
        ok = movePointer(action, window, &error);
    } else if (kind == QLatin1String("pointer_click")) {
        ok = clickPointer(action, window, &error);
    } else if (kind == QLatin1String("pointer_drag")) {
        ok = dragPointer(action, window, &error);
    } else if (kind == QLatin1String("pointer_scroll")) {
        ok = scrollPointer(action, window, &error);
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

bool SeatgeistAgentSeatPlugin::bindTarget(
    Window *window,
    const QPointF &localPosition,
    QString *error)
{
    if (!window || !window->surface()) {
        *error = QStringLiteral("target surface is unavailable");
        return false;
    }
    const QRectF frame = window->frameGeometry();
    if (!QRectF(QPointF(0, 0), frame.size()).contains(localPosition)) {
        *error = QStringLiteral("window-local pointer coordinate is outside the target");
        return false;
    }
    const QPointF globalPosition = frame.topLeft() + localPosition;
    setTimestamp();
    m_seat->setFocusedKeyboardSurface(window->surface());
    m_seat->notifyPointerEnter(
        window->surface(),
        globalPosition,
        window->inputTransformation());
    m_seat->notifyPointerFrame();
    m_target = window;
    m_localPointerPosition = localPosition;
    return true;
}

bool SeatgeistAgentSeatPlugin::sendKeyCombo(
    const QJsonObject &action,
    Window *window,
    QString *error)
{
    if (!m_xkbState) {
        *error = QStringLiteral("agent-seat keymap is unavailable");
        return false;
    }
    const QJsonArray keycodes = action.value(QStringLiteral("keycodes")).toArray();
    if (keycodes.isEmpty() || keycodes.size() > 8) {
        *error = QStringLiteral("key combo must contain between 1 and 8 keys");
        return false;
    }
    const QPointF position = m_target == window
        ? m_localPointerPosition
        : QPointF(window->width() / 2.0, window->height() / 2.0);
    if (!bindTarget(window, position, error)) {
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
        setTimestamp();
        m_seat->notifyKeyboardKey(
            code,
            KeyboardKeyState::Pressed,
            waylandServer()->display()->nextSerial());
        xkb_state_update_key(m_xkbState, code + 8, XKB_KEY_DOWN);
        updateKeyboardModifiers();
    }
    for (auto it = codes.crbegin(); it != codes.crend(); ++it) {
        const quint32 code = *it;
        setTimestamp();
        m_seat->notifyKeyboardKey(
            code,
            KeyboardKeyState::Released,
            waylandServer()->display()->nextSerial());
        xkb_state_update_key(m_xkbState, code + 8, XKB_KEY_UP);
        updateKeyboardModifiers();
    }
    return true;
}

bool SeatgeistAgentSeatPlugin::sendKeySequence(
    const QJsonObject &action,
    Window *window,
    QString *error)
{
    if (!m_xkbState) {
        *error = QStringLiteral("agent-seat keymap is unavailable");
        return false;
    }
    const QJsonArray chords = action.value(QStringLiteral("chords")).toArray();
    if (chords.isEmpty() || chords.size() > 8192) {
        *error = QStringLiteral("key sequence must contain between 1 and 8192 chords");
        return false;
    }
    const QPointF position = m_target == window
        ? m_localPointerPosition
        : QPointF(window->width() / 2.0, window->height() / 2.0);
    if (!bindTarget(window, position, error)) {
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
            setTimestamp();
            m_seat->notifyKeyboardKey(
                code,
                KeyboardKeyState::Pressed,
                waylandServer()->display()->nextSerial());
            xkb_state_update_key(m_xkbState, code + 8, XKB_KEY_DOWN);
            updateKeyboardModifiers();
        }
        for (auto it = codes.crbegin(); it != codes.crend(); ++it) {
            const quint32 code = *it;
            setTimestamp();
            m_seat->notifyKeyboardKey(
                code,
                KeyboardKeyState::Released,
                waylandServer()->display()->nextSerial());
            xkb_state_update_key(m_xkbState, code + 8, XKB_KEY_UP);
            updateKeyboardModifiers();
        }
    }
    return true;
}

bool SeatgeistAgentSeatPlugin::movePointer(
    const QJsonObject &action,
    Window *window,
    QString *error)
{
    if (!finiteNumber(action.value(QStringLiteral("x")))
        || !finiteNumber(action.value(QStringLiteral("y")))) {
        *error = QStringLiteral("pointer coordinates must be finite numbers");
        return false;
    }
    return bindTarget(
        window,
        QPointF(
            action.value(QStringLiteral("x")).toDouble(),
            action.value(QStringLiteral("y")).toDouble()),
        error);
}

bool SeatgeistAgentSeatPlugin::clickPointer(
    const QJsonObject &action,
    Window *window,
    QString *error)
{
    if (!movePointer(action, window, error)) {
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
        setTimestamp();
        m_seat->notifyPointerButton(
            static_cast<quint32>(button),
            PointerButtonState::Pressed);
        m_seat->notifyPointerFrame();
        setTimestamp();
        m_seat->notifyPointerButton(
            static_cast<quint32>(button),
            PointerButtonState::Released);
        m_seat->notifyPointerFrame();
    }
    return true;
}

bool SeatgeistAgentSeatPlugin::dragPointer(
    const QJsonObject &action,
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
    if (!bindTarget(window, QPointF(fromX.toDouble(), fromY.toDouble()), error)) {
        return false;
    }
    setTimestamp();
    m_seat->notifyPointerButton(
        static_cast<quint32>(button),
        PointerButtonState::Pressed);
    m_seat->notifyPointerFrame();
    if (!bindTarget(window, QPointF(toX.toDouble(), toY.toDouble()), error)) {
        m_seat->notifyPointerButton(
            static_cast<quint32>(button),
            PointerButtonState::Released);
        m_seat->notifyPointerFrame();
        return false;
    }
    setTimestamp();
    m_seat->notifyPointerButton(
        static_cast<quint32>(button),
        PointerButtonState::Released);
    m_seat->notifyPointerFrame();
    return true;
}

bool SeatgeistAgentSeatPlugin::scrollPointer(
    const QJsonObject &action,
    Window *window,
    QString *error)
{
    const int vertical = action.value(QStringLiteral("vertical")).toInt();
    const int horizontal = action.value(QStringLiteral("horizontal")).toInt();
    if (vertical == 0 && horizontal == 0) {
        *error = QStringLiteral("scroll delta must be non-zero");
        return false;
    }
    const QPointF position = m_target == window
        ? m_localPointerPosition
        : QPointF(window->width() / 2.0, window->height() / 2.0);
    if (!bindTarget(window, position, error)) {
        return false;
    }
    setTimestamp();
    if (vertical != 0) {
        m_seat->notifyPointerAxis(
            Qt::Vertical,
            vertical * 15.0,
            vertical * 120,
            PointerAxisSource::Wheel);
    }
    if (horizontal != 0) {
        m_seat->notifyPointerAxis(
            Qt::Horizontal,
            horizontal * 15.0,
            horizontal * 120,
            PointerAxisSource::Wheel);
    }
    m_seat->notifyPointerFrame();
    return true;
}

void SeatgeistAgentSeatPlugin::updateKeyboardModifiers()
{
    m_seat->notifyKeyboardModifiers(
        xkb_state_serialize_mods(m_xkbState, XKB_STATE_MODS_DEPRESSED),
        xkb_state_serialize_mods(m_xkbState, XKB_STATE_MODS_LATCHED),
        xkb_state_serialize_mods(m_xkbState, XKB_STATE_MODS_LOCKED),
        xkb_state_serialize_layout(m_xkbState, XKB_STATE_LAYOUT_EFFECTIVE));
}

void SeatgeistAgentSeatPlugin::setTimestamp()
{
    m_seat->setTimestamp(std::chrono::microseconds(m_monotonicClock.nsecsElapsed() / 1000));
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

void SeatgeistAgentSeatPlugin::clearTarget()
{
    if (!m_seat) {
        return;
    }
    m_seat->setFocusedKeyboardSurface(nullptr);
    m_seat->notifyPointerLeave();
    m_seat->notifyPointerFrame();
    m_target = nullptr;
}

} // namespace KWin

#include "moc_agentseatplugin.cpp"
