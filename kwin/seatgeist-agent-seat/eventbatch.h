// SPDX-License-Identifier: MIT OR Apache-2.0
#pragma once

#include <QJsonObject>
#include <QPointF>
#include <QSizeF>
#include <QString>
#include <QVector>

namespace KWin
{
struct SeatEvent {
    enum Kind { Key, Motion, Button, Axis } kind;
    quint32 code = 0;
    bool pressed = false;
    QPointF position;
    Qt::Orientation axis = Qt::Vertical;
    qreal delta = 0;
    qint32 deltaV120 = 0;
};

// The entire batch is decoded and validated before any seat focus or input.
bool decodeSeatEvents(const QJsonObject &action, const QSizeF &targetSize,
                      QVector<SeatEvent> *events, QString *error);
}
