// SPDX-License-Identifier: MIT OR Apache-2.0

#include "activityplugin.h"

#include <core/inputdevice.h>
#include <input.h>
#include <input_event.h>

#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusPendingCall>
#include <QDBusServiceWatcher>
#include <QJsonDocument>
#include <QJsonObject>

namespace
{

constexpr auto Service = "org.seatgeist.KWinBridge";
constexpr auto Path = "/org/seatgeist/KWinBridge1";
constexpr auto Interface = "org.seatgeist.KWinBridge1";
constexpr auto Backend = "kwin_input_spy_v1";
constexpr qint64 PointerPublishIntervalMs = 20;

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

SeatgeistActivityPlugin::SeatgeistActivityPlugin()
{
    m_monotonicClock.start();
    input()->installInputEventSpy(this);
    m_serviceWatcher = std::make_unique<QDBusServiceWatcher>(
        QString::fromLatin1(Service),
        QDBusConnection::sessionBus(),
        QDBusServiceWatcher::WatchForRegistration,
        this);
    connect(
        m_serviceWatcher.get(),
        &QDBusServiceWatcher::serviceRegistered,
        this,
        [this](const QString &) { registerBackend(); });
    registerBackend();
}

SeatgeistActivityPlugin::~SeatgeistActivityPlugin() = default;

void SeatgeistActivityPlugin::registerBackend()
{
    auto message = methodCall(QStringLiteral("RegisterInputActivityBackend"));
    message << QString::fromLatin1(Backend);
    QDBusConnection::sessionBus().asyncCall(message);
}

QString SeatgeistActivityPlugin::provenanceFor(const InputDevice *device) const
{
    if (!device) {
        return QStringLiteral("unknown");
    }
    const QString name = device->name();
    if (name.startsWith(QStringLiteral("Seatgeist "), Qt::CaseInsensitive)
        || name.startsWith(QStringLiteral("Seatgeist Virtual "), Qt::CaseInsensitive)) {
        return QStringLiteral("seatgeist_injected");
    }
    const QString sysPath = device->sysPath();
    if (sysPath.isEmpty() || sysPath.contains(QStringLiteral("/virtual/"))) {
        return QStringLiteral("unknown");
    }
    return QStringLiteral("trusted_physical");
}

void SeatgeistActivityPlugin::publish(
    const QString &eventClass,
    const InputDevice *device,
    bool throttle)
{
    const QString provenance = provenanceFor(device);
    const qint64 now = m_monotonicClock.elapsed();
    const QString throttleKey = eventClass + QLatin1Char(':') + provenance;
    if (throttle && m_lastPublished.contains(throttleKey)
        && now - m_lastPublished.value(throttleKey) < PointerPublishIntervalMs) {
        return;
    }
    m_lastPublished.insert(throttleKey, now);

    const QJsonObject payload{
        {QStringLiteral("backend"), QString::fromLatin1(Backend)},
        {QStringLiteral("seat"), QStringLiteral("default")},
        {QStringLiteral("class"), eventClass},
        {QStringLiteral("provenance"), provenance},
        {QStringLiteral("monotonic_ms"), now},
    };
    auto message = methodCall(QStringLiteral("UpdateInputActivity"));
    message << QString::fromUtf8(QJsonDocument(payload).toJson(QJsonDocument::Compact));
    QDBusConnection::sessionBus().asyncCall(message);
}

void SeatgeistActivityPlugin::pointerMotion(PointerMotionEvent *event)
{
    publish(QStringLiteral("pointer"), event->device, true);
}

void SeatgeistActivityPlugin::pointerButton(PointerButtonEvent *event)
{
    publish(QStringLiteral("pointer"), event->device, false);
}

void SeatgeistActivityPlugin::pointerAxis(PointerAxisEvent *event)
{
    publish(QStringLiteral("pointer"), event->device, true);
}

void SeatgeistActivityPlugin::keyboardKey(KeyboardKeyEvent *event)
{
    publish(QStringLiteral("keyboard"), event->device, false);
}

void SeatgeistActivityPlugin::touchDown(TouchDownEvent *)
{
    publish(QStringLiteral("touch"), nullptr, false);
}

void SeatgeistActivityPlugin::touchMotion(TouchMotionEvent *)
{
    publish(QStringLiteral("touch"), nullptr, true);
}

void SeatgeistActivityPlugin::touchUp(TouchUpEvent *)
{
    publish(QStringLiteral("touch"), nullptr, false);
}

} // namespace KWin

#include "moc_activityplugin.cpp"
