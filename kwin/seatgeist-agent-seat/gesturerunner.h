// SPDX-License-Identifier: MIT OR Apache-2.0
#pragma once
#include "eventbatch.h"
#include <QSet>
#include <functional>
#include <optional>
namespace KWin {
// Finite scheduler only; hooks retain seat ownership and perform actual checks.
class GestureRunner {
public:
  struct Hooks {
    std::function<bool()> valid;
    std::function<void()> authorize;
    std::function<bool(const SeatEvent &)> condition;
    std::function<bool(const SeatEvent &)> deliver;
    std::function<void(const QSet<quint32> &, const QSet<quint32> &)> release;
    std::function<void(bool, const QString &)> finished;
  };
  GestureRunner(QVector<SeatEvent>, Hooks);
  ~GestureRunner();
  void tick(qint64 now);
  void authorized(bool allowed, qint64 now);
  void cancel(const QString &reason = QStringLiteral("gesture cancelled"));

private:
  enum Phase { Ready, Authorizing, Waiting, Terminal } m_phase = Ready;
  QVector<SeatEvent> m_steps;
  Hooks m_hooks;
  qsizetype m_index = 0;
  QSet<quint32> m_keys, m_buttons;
  qint64 m_deadline = -1, m_waitUntil = 0, m_lastAuthorization = -1;
  bool m_recheck = false;
  std::optional<SeatEvent> m_gate;
  void finish(bool, const QString &);
};
} // namespace KWin
