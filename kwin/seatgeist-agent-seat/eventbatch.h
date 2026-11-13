// SPDX-License-Identifier: MIT OR Apache-2.0
#pragma once

#include <QJsonArray>
#include <QJsonObject>
#include <QPointF>
#include <QSizeF>
#include <QString>
#include <QVector>

namespace KWin {
struct SeatEvent {
  enum Kind {
    Key,
    Motion,
    RelativeMotion,
    AltPointClick,
    Button,
    Axis,
    Gesture,
    State,
    Delay,
    Condition
  } kind;
  QJsonArray steps;
  QString condition;
  QString source;
  bool inverted = false;
  qint64 durationMs = 0;
  quint32 code = 0;
  bool pressed = false;
  QPointF position;
  Qt::Orientation axis = Qt::Vertical;
  qreal delta = 0;
  qint32 deltaV120 = 0;
};

// The entire batch is decoded and validated before any seat focus or input.
bool decodeGestureSteps(const QJsonArray &, const QSizeF &,
                        QVector<SeatEvent> *, QString *);
bool decodeSeatEvents(const QJsonObject &action, const QSizeF &targetSize,
                      QVector<SeatEvent> *events, QString *error);
} // namespace KWin
