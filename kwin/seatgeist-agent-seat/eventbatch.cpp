// SPDX-License-Identifier: MIT OR Apache-2.0
#include "eventbatch.h"

#include <QJsonArray>
#include <QSet>
#include <linux/input-event-codes.h>
#include <cmath>
#include <limits>

namespace KWin
{
namespace
{
bool integer(const QJsonValue &value, double minimum, double maximum)
{
    const double number = value.toDouble();
    return value.isDouble() && std::isfinite(number) && std::floor(number) == number
        && number >= minimum && number <= maximum;
}
}

bool decodeSeatEvents(const QJsonObject &action, const QSizeF &targetSize,
                      QVector<SeatEvent> *events, QString *error)
{
    events->clear();
    const auto fail = [error](const char *message) {
        *error = QString::fromLatin1(message);
        return false;
    };
    if (action.value("action") != QLatin1String("events")
        || !integer(action.value("protocol"), 1, 1)) {
        return fail("unsupported seat-host event protocol");
    }
    if (!integer(action.value("expires_at_unix_ms"), 1, 9007199254740991.0)) {
        return fail("seat-host action requires a finite delivery deadline");
    }
    if (!action.value("events").isArray()) {
        return fail("seat-host events must be an array");
    }
    const auto array = action.value("events").toArray();
    if (array.isEmpty() || array.size() > 32768) {
        return fail("seat-host event count is outside the safety bound");
    }
    QVector<SeatEvent> decoded;
    QSet<quint32> keys;
    QSet<quint32> buttons;
    for (const auto value : array) {
        const auto object = value.toObject();
        const auto op = object.value("op").toString();
        SeatEvent event{};
        if (op == QLatin1String("key") || op == QLatin1String("button")) {
            event.kind = op == QLatin1String("key") ? SeatEvent::Key : SeatEvent::Button;
            if (!integer(object.value("code"), 1, KEY_MAX) || !object.value("pressed").isBool()) {
                return fail("invalid seat-host key/button event");
            }
            event.code = object.value("code").toInt();
            event.pressed = object.value("pressed").toBool();
            if (event.kind == SeatEvent::Button && event.code != BTN_LEFT
                && event.code != BTN_MIDDLE && event.code != BTN_RIGHT) {
                return fail("unsupported seat-host pointer button");
            }
            auto &pressed = event.kind == SeatEvent::Key ? keys : buttons;
            if (pressed.contains(event.code) == event.pressed) {
                return fail("unbalanced seat-host key/button transition");
            }
            if (event.pressed) { pressed.insert(event.code); }
            else { pressed.remove(event.code); }
        } else if (op == QLatin1String("relative_motion")) {
            // One finite relative event only: never mix lock-dependent motion
            // with focus changes or a partially deliverable keyboard batch.
            const auto dx = object.value("dx");
            const auto dy = object.value("dy");
            if (array.size() != 1 || !dx.isDouble() || !dy.isDouble()
                || !std::isfinite(dx.toDouble()) || !std::isfinite(dy.toDouble())
                || std::abs(dx.toDouble()) > 4096 || std::abs(dy.toDouble()) > 4096
                || (dx.toDouble() == 0 && dy.toDouble() == 0)) {
                return fail("invalid finite relative pointer delta");
            }
            event.kind = SeatEvent::RelativeMotion;
            event.position = QPointF(dx.toDouble(), dy.toDouble());
        } else if (op == QLatin1String("motion")) {
            event.kind = SeatEvent::Motion;
            const auto x = object.value("x");
            const auto y = object.value("y");
            if (!x.isDouble() || !y.isDouble() || !std::isfinite(x.toDouble()) || !std::isfinite(y.toDouble())
                || x.toDouble() < 0 || y.toDouble() < 0
                || x.toDouble() >= targetSize.width() || y.toDouble() >= targetSize.height()) {
                return fail("seat-host motion is outside the target client surface");
            }
            event.position = QPointF(x.toDouble(), y.toDouble());
        } else if (op == QLatin1String("axis")) {
            event.kind = SeatEvent::Axis;
            const auto axis = object.value("axis").toString();
            if (axis != QLatin1String("vertical") && axis != QLatin1String("horizontal")) {
                return fail("unsupported seat-host axis");
            }
            event.axis = axis == QLatin1String("vertical") ? Qt::Vertical : Qt::Horizontal;
            const auto delta = object.value("delta");
            if (!delta.isDouble() || !std::isfinite(delta.toDouble()) || std::abs(delta.toDouble()) > 1000000
                || !integer(object.value("delta_v120"), std::numeric_limits<qint32>::min(), std::numeric_limits<qint32>::max())) {
                return fail("invalid seat-host axis delta");
            }
            event.delta = delta.toDouble();
            event.deltaV120 = object.value("delta_v120").toInt();
        } else {
            return fail("unsupported seat-host event opcode");
        }
        decoded.push_back(event);
    }
    if (!keys.isEmpty() || !buttons.isEmpty()) {
        return fail("seat-host batch leaves keys or buttons pressed");
    }
    *events = std::move(decoded);
    return true;
}
}
