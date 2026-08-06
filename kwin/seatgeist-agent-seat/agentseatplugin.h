// SPDX-License-Identifier: MIT OR Apache-2.0
#pragma once

#include <plugin.h>

#include <QElapsedTimer>
#include <QJsonObject>
#include <QPointF>
#include <QPointer>

#include <memory>
#include <map>

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
    struct AgentLane {
        std::unique_ptr<SeatInterface> seat;
        QPointer<Window> target;
        QPointF localPointerPosition;
        xkb_state *xkbState = nullptr;
        qint64 lastUsedMs = 0;
    };

    void registerBackend();
    void poll();
    void handlePendingAction(QDBusPendingCallWatcher *watcher);
    void executeAction(const QJsonObject &action);
    void complete(const QString &id, bool ok, const QString &error = {});
    void clearLaneTarget(AgentLane &lane);
    void clearLanes();

    Window *resolveNativeWaylandWindow(const QString &windowId, QString *error) const;
    AgentLane *laneFor(const QString &laneId, QString *error);
    bool initializeLane(AgentLane &lane, QString *error);
    bool bindTarget(AgentLane &lane, Window *window, const QPointF &localPosition, QString *error);
    bool sendKeyCombo(const QJsonObject &action, AgentLane &lane, Window *window, QString *error);
    bool sendKeySequence(const QJsonObject &action, AgentLane &lane, Window *window, QString *error);
    bool movePointer(const QJsonObject &action, AgentLane &lane, Window *window, QString *error);
    bool clickPointer(const QJsonObject &action, AgentLane &lane, Window *window, QString *error);
    bool dragPointer(const QJsonObject &action, AgentLane &lane, Window *window, QString *error);
    bool scrollPointer(const QJsonObject &action, AgentLane &lane, Window *window, QString *error);
    bool initializeKeymap(QString *error);
    void updateKeyboardModifiers(AgentLane &lane);
    void setTimestamp(AgentLane &lane);

    std::unique_ptr<QDBusServiceWatcher> m_serviceWatcher;
    QTimer *m_pollTimer = nullptr;
    QElapsedTimer m_monotonicClock;
    bool m_serviceAvailable = false;
    bool m_pollInFlight = false;
    std::map<QString, std::unique_ptr<AgentLane>> m_lanes;
    quint64 m_nextLaneIndex = 0;
    xkb_context *m_xkbContext = nullptr;
    xkb_keymap *m_xkbKeymap = nullptr;
};

} // namespace KWin
