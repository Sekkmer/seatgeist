// SPDX-License-Identifier: MIT OR Apache-2.0
#include "gesturerunner.h"
namespace KWin {
GestureRunner::GestureRunner(QVector<SeatEvent> steps, Hooks hooks)
    : m_steps(std::move(steps)), m_hooks(std::move(hooks)) {}
GestureRunner::~GestureRunner() {
  cancel(QStringLiteral("gesture owner destroyed"));
}
void GestureRunner::finish(bool ok, const QString &reason) {
  if (m_phase == Terminal)
    return;
  m_phase = Terminal;
  m_hooks.release(m_keys, m_buttons);
  m_keys.clear();
  m_buttons.clear();
  m_hooks.finished(ok, reason);
}
void GestureRunner::cancel(const QString &reason) { finish(false, reason); }
void GestureRunner::tick(qint64 now) {
  if (m_phase == Terminal)
    return;
  if (m_deadline < 0)
    m_deadline = now + 2500;
  if (!m_hooks.valid() || now >= m_deadline) {
    finish(false,
           QStringLiteral("gesture target/lease lost or deadline expired"));
    return;
  }
  if (m_phase == Authorizing)
    return;
  if (m_phase == Waiting) {
    const auto &e = m_steps[m_index];
    const bool delay = e.kind == SeatEvent::Delay;
    if (!delay && now >= m_waitUntil) {
      finish(false, QStringLiteral("gesture client condition timed out"));
      return;
    }
    const bool ready = delay ? now >= m_waitUntil : m_hooks.condition(e);
    if (!delay && now >= m_waitUntil && !ready) {
      finish(false, QStringLiteral("gesture client condition timed out"));
      return;
    }
    if (ready) {
      if (!delay)
        m_gate = e;
      ++m_index;
      m_phase = Ready;
    } else if (now - m_lastAuthorization < 50)
      return;
  }
  if (m_index == m_steps.size()) {
    finish(true, {});
    return;
  }
  m_recheck = m_phase == Waiting;
  m_phase = Authorizing;
  m_hooks.authorize();
}
void GestureRunner::authorized(bool allowed, qint64 now) {
  if (m_phase != Authorizing)
    return;
  if (!allowed || !m_hooks.valid() || now >= m_deadline) {
    finish(false, QStringLiteral("gesture authorization cancelled or expired"));
    return;
  }
  m_lastAuthorization = now;
  if (m_recheck) {
    m_phase = Waiting;
    return;
  }
  const auto &e = m_steps[m_index];
  if (m_gate && !m_hooks.condition(*m_gate)) {
    finish(false, QStringLiteral(
                      "gesture client condition changed during authorization"));
    return;
  }
  m_gate.reset();
  if (e.kind == SeatEvent::Delay || e.kind == SeatEvent::Condition) {
    m_waitUntil = now + e.durationMs;
    m_phase = Waiting;
    return;
  }
  if (!m_hooks.deliver(e)) {
    finish(false, QStringLiteral(
                      "gesture input binding/lock changed before delivery"));
    return;
  }
  if (e.kind == SeatEvent::Key) {
    if (e.pressed)
      m_keys.insert(e.code);
    else
      m_keys.remove(e.code);
  }
  if (e.kind == SeatEvent::Button) {
    if (e.pressed)
      m_buttons.insert(e.code);
    else
      m_buttons.remove(e.code);
  }
  ++m_index;
  m_phase = Ready;
}
} // namespace KWin
