// SPDX-License-Identifier: MIT OR Apache-2.0
#pragma once

#include <input_event_spy.h>
#include <plugin.h>

#include <QElapsedTimer>
#include <QHash>

#include <memory>

class QDBusServiceWatcher;

namespace KWin
{

class InputDevice;

class SeatgeistActivityPlugin final : public Plugin, public InputEventSpy
{
    Q_OBJECT

public:
    SeatgeistActivityPlugin();
    ~SeatgeistActivityPlugin() override;

    void pointerMotion(PointerMotionEvent *event) override;
    void pointerButton(PointerButtonEvent *event) override;
    void pointerAxis(PointerAxisEvent *event) override;
    void keyboardKey(KeyboardKeyEvent *event) override;
    void touchDown(TouchDownEvent *event) override;
    void touchMotion(TouchMotionEvent *event) override;
    void touchUp(TouchUpEvent *event) override;

private:
    QString provenanceFor(const InputDevice *device) const;
    void publish(const QString &eventClass, const InputDevice *device, bool throttle);
    void registerBackend();

    QElapsedTimer m_monotonicClock;
    QHash<QString, qint64> m_lastPublished;
    std::unique_ptr<QDBusServiceWatcher> m_serviceWatcher;
};

} // namespace KWin
