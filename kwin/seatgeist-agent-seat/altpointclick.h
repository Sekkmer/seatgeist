// SPDX-License-Identifier: MIT OR Apache-2.0
#pragma once
#include "seatbindings.h"
#include <functional>

namespace KWin {
// One finite gesture. No general held-key handle is exported. The owner drives
// tick only after returning to the compositor loop; tests can drive fake time.
class AltPointClick {
public:
    struct Hooks {
        std::function<bool()> valid;
        std::function<void(bool)> alt;
        std::function<void()> authorize;
        std::function<bool()> click;
        std::function<void(bool, const QString &)> finished;
    };
    AltPointClick(SeatBindings &, SeatInterface *, SurfaceInterface *, Hooks);
    ~AltPointClick();
    void start(qint64 nowMs);
    void tick(qint64 nowMs);
    void authorized(bool allowed, qint64 nowMs);
    void cancel(const QString &reason = QStringLiteral("Alt-point click cancelled"));
private:
    enum State { Initial, WaitingUnlock, Authorizing, Terminal } m_state = Initial;
    bool valid(qint64 nowMs) const;
    bool unlocked() const;
    void finish(bool ok, const QString &reason);
    SeatBindings &m_bindings;
    SeatInterface *m_seat;
    QPointer<SurfaceInterface> m_surface;
    std::shared_ptr<SeatBindings::LockReleaseObservation> m_release;
    Hooks m_hooks;
    qint64 m_deadline = 0;
    bool m_altDown = false;
};
}
