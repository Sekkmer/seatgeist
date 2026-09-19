// SPDX-License-Identifier: MIT OR Apache-2.0
#pragma once

#include <plugin.h>
#include <input_event_spy.h>
#include "eventbatch.h"

#include <QElapsedTimer>
#include <QJsonObject>
#include <QPointF>
#include <QPointer>

#include <memory>
#include <map>
#include <vector>

class QDBusPendingCallWatcher;
class QDBusServiceWatcher;
class QTimer;

struct xkb_context;
struct xkb_keymap;
struct xkb_state;

namespace KWin
{

class SeatInterface;
class SeatBindings;
class SurfaceInterface;
class Window;

class SeatgeistAgentSeatPlugin final : public Plugin, public InputEventSpy
{
    Q_OBJECT

public:
    SeatgeistAgentSeatPlugin();
    ~SeatgeistAgentSeatPlugin() override;

    void keyboardKey(KeyboardKeyEvent *) override;
    void pointerMotion(PointerMotionEvent *) override;
    void pointerButton(PointerButtonEvent *) override;
    void pointerAxis(PointerAxisEvent *) override;
    void touchDown(TouchDownEvent *) override;
    void touchMotion(TouchMotionEvent *) override;
    void touchUp(TouchUpEvent *) override;

private:
    struct AgentLane {
        std::unique_ptr<SeatInterface> seat;
        QPointer<Window> target;
        QPointF localPointerPosition;
        xkb_state *xkbState = nullptr;
    };

    void registerBackend();
    void poll();
    void handlePendingAction(QDBusPendingCallWatcher *watcher);
    void executeAction(const QJsonObject &action);
    void dispatchAction(const QJsonObject &action, AgentLane &lane, Window *window);
    void resumePendingAction();
    void cancelPendingFor(Window *window);
    void complete(const QString &id, bool ok, const QString &error = {});
    void clearLaneTarget(AgentLane &lane);
    void clearLanes();
    void detachDaemon();

    Window *resolveNativeWaylandWindow(const QString &windowId, QString *error) const;
    bool validateClientFocusIsolation(Window *window, QString *error) const;
    AgentLane *laneFor(const QString &laneId, QString *error);
    bool initializeLane(AgentLane &lane, QString *error);
    bool bindTarget(AgentLane &lane, Window *window, const QPointF &localPosition, QString *error);
    bool initializeKeymap(QString *error);
    void updateKeyboardModifiers(AgentLane &lane);
    void setTimestamp(AgentLane &lane);

    std::unique_ptr<QDBusServiceWatcher> m_serviceWatcher;
    QTimer *m_pollTimer = nullptr;
    QElapsedTimer m_monotonicClock;
    bool m_serviceAvailable = false;
    bool m_pollInFlight = false;
    quint64 m_serviceGeneration = 0;
    std::unique_ptr<SeatBindings> m_bindings;
    QTimer *m_bindingTimer = nullptr;
    QJsonObject m_pendingAction;
    QVector<SeatEvent> m_pendingEvents;
    QPointer<Window> m_pendingWindow;
    QPointer<SurfaceInterface> m_pendingSurface;
    qint64 m_bindingDeadlineMs = 0;
    bool m_pendingCancelled = false;
    std::map<qint32, QPointer<Window>> m_touchTargets;
    std::map<QString, std::unique_ptr<AgentLane>> m_lanes;
    std::vector<std::unique_ptr<AgentLane>> m_idleLanes;
    quint64 m_nextLaneIndex = 0;
    xkb_context *m_xkbContext = nullptr;
    xkb_keymap *m_xkbKeymap = nullptr;
};

} // namespace KWin
