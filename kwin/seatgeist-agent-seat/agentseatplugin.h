// SPDX-License-Identifier: MIT OR Apache-2.0
#pragma once

#include <plugin.h>

#include <QElapsedTimer>
#include <QJsonObject>
#include <QPointF>
#include <QPointer>

#include <memory>

class QDBusPendingCallWatcher;
class QDBusServiceWatcher;
class QTimer;

struct xkb_context;
struct xkb_keymap;
struct xkb_state;

namespace KWin
{

class SeatInterface;
class Window;

class SeatgeistAgentSeatPlugin final : public Plugin
{
    Q_OBJECT

public:
    SeatgeistAgentSeatPlugin();
    ~SeatgeistAgentSeatPlugin() override;

private:
    void registerBackend();
    void poll();
    void handlePendingAction(QDBusPendingCallWatcher *watcher);
    void executeAction(const QJsonObject &action);
    void complete(const QString &id, bool ok, const QString &error = {});
    void clearTarget();

    Window *resolveNativeWaylandWindow(const QString &windowId, QString *error) const;
    bool bindTarget(Window *window, const QPointF &localPosition, QString *error);
    bool sendKeyCombo(const QJsonObject &action, Window *window, QString *error);
    bool sendKeySequence(const QJsonObject &action, Window *window, QString *error);
    bool movePointer(const QJsonObject &action, Window *window, QString *error);
    bool clickPointer(const QJsonObject &action, Window *window, QString *error);
    bool dragPointer(const QJsonObject &action, Window *window, QString *error);
    bool scrollPointer(const QJsonObject &action, Window *window, QString *error);
    bool initializeKeymap(QString *error);
    void updateKeyboardModifiers();
    void setTimestamp();

    std::unique_ptr<SeatInterface> m_seat;
    std::unique_ptr<QDBusServiceWatcher> m_serviceWatcher;
    QTimer *m_pollTimer = nullptr;
    QElapsedTimer m_monotonicClock;
    bool m_pollInFlight = false;
    QPointer<Window> m_target;
    QPointF m_localPointerPosition;
    xkb_context *m_xkbContext = nullptr;
    xkb_keymap *m_xkbKeymap = nullptr;
    xkb_state *m_xkbState = nullptr;
};

} // namespace KWin
