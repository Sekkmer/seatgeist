// SPDX-License-Identifier: MIT OR Apache-2.0
#include "eventbatch.h"

#include <QJsonArray>
#include <QSet>
#include <cmath>
#include <limits>
#include <linux/input-event-codes.h>

namespace KWin {
namespace {
bool integer(const QJsonValue &value, double minimum, double maximum) {
  const double number = value.toDouble();
  return value.isDouble() && std::isfinite(number) &&
         std::floor(number) == number && number >= minimum && number <= maximum;
}
} // namespace

bool decodeSeatEvents(const QJsonObject &action, const QSizeF &targetSize,
                      QVector<SeatEvent> *events, QString *error) {
  events->clear();
  const auto fail = [error](const char *message) {
    *error = QString::fromLatin1(message);
    return false;
  };
  if (action.value("action") != QLatin1String("events") ||
      !integer(action.value("protocol"), 1, 1)) {
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
    if (op == QLatin1String("gesture")) {
      QVector<SeatEvent> steps;
      if (array.size() != 1 || !object.value("steps").isArray() ||
          !decodeGestureSteps(object.value("steps").toArray(), targetSize,
                              &steps, error)) {
        return false;
      }
      event.kind = SeatEvent::Gesture;
      event.steps = object.value("steps").toArray();
    } else if (op == QLatin1String("state")) {
      if (array.size() != 1) {
        return fail("seat state observation must be standalone");
      }
      event.kind = SeatEvent::State;
    } else if (op == QLatin1String("key") || op == QLatin1String("button")) {
      event.kind =
          op == QLatin1String("key") ? SeatEvent::Key : SeatEvent::Button;
      if (!integer(object.value("code"), 1, KEY_MAX) ||
          !object.value("pressed").isBool()) {
        return fail("invalid seat-host key/button event");
      }
      event.code = object.value("code").toInt();
      event.pressed = object.value("pressed").toBool();
      if (event.kind == SeatEvent::Button && event.code != BTN_LEFT &&
          event.code != BTN_MIDDLE && event.code != BTN_RIGHT) {
        return fail("unsupported seat-host pointer button");
      }
      auto &pressed = event.kind == SeatEvent::Key ? keys : buttons;
      if (pressed.contains(event.code) == event.pressed) {
        return fail("unbalanced seat-host key/button transition");
      }
      if (event.pressed) {
        pressed.insert(event.code);
      } else {
        pressed.remove(event.code);
      }
    } else if (op == QLatin1String("relative_motion")) {
      // One finite relative event only: never mix lock-dependent motion
      // with focus changes or a partially deliverable keyboard batch.
      const auto dx = object.value("dx");
      const auto dy = object.value("dy");
      if (array.size() != 1 || !dx.isDouble() || !dy.isDouble() ||
          !std::isfinite(dx.toDouble()) || !std::isfinite(dy.toDouble()) ||
          std::abs(dx.toDouble()) > 4096 || std::abs(dy.toDouble()) > 4096 ||
          (dx.toDouble() == 0 && dy.toDouble() == 0)) {
        return fail("invalid finite relative pointer delta");
      }
      event.kind = SeatEvent::RelativeMotion;
      event.position = QPointF(dx.toDouble(), dy.toDouble());
    } else if (op == QLatin1String("alt_point_click")) {
      const auto x = object.value("x");
      const auto y = object.value("y");
      if (array.size() != 1 || !x.isDouble() || !y.isDouble() ||
          !std::isfinite(x.toDouble()) || !std::isfinite(y.toDouble()) ||
          x.toDouble() < 0 || y.toDouble() < 0 ||
          x.toDouble() >= targetSize.width() ||
          y.toDouble() >= targetSize.height()) {
        return fail("invalid standalone Alt-point click");
      }
      event.kind = SeatEvent::AltPointClick;
      event.position = QPointF(x.toDouble(), y.toDouble());
    } else if (op == QLatin1String("motion")) {
      event.kind = SeatEvent::Motion;
      const auto x = object.value("x");
      const auto y = object.value("y");
      if (!x.isDouble() || !y.isDouble() || !std::isfinite(x.toDouble()) ||
          !std::isfinite(y.toDouble()) || x.toDouble() < 0 ||
          y.toDouble() < 0 || x.toDouble() >= targetSize.width() ||
          y.toDouble() >= targetSize.height()) {
        return fail("seat-host motion is outside the target client surface");
      }
      event.position = QPointF(x.toDouble(), y.toDouble());
    } else if (op == QLatin1String("axis")) {
      event.kind = SeatEvent::Axis;
      const auto axis = object.value("axis").toString();
      if (axis != QLatin1String("vertical") &&
          axis != QLatin1String("horizontal")) {
        return fail("unsupported seat-host axis");
      }
      event.axis =
          axis == QLatin1String("vertical") ? Qt::Vertical : Qt::Horizontal;
      const auto delta = object.value("delta");
      if (!delta.isDouble() || !std::isfinite(delta.toDouble()) ||
          std::abs(delta.toDouble()) > 1000000 ||
          !integer(object.value("delta_v120"),
                   std::numeric_limits<qint32>::min(),
                   std::numeric_limits<qint32>::max())) {
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
bool decodeGestureSteps(const QJsonArray &array, const QSizeF &size,
                        QVector<SeatEvent> *events, QString *error) {
  events->clear();
  QVector<SeatEvent> decoded;
  QSet<quint32> keys, buttons;
  qint64 budget = 0;
  bool input = false;
  auto fail = [&](const char *message) {
    *error = QString::fromLatin1(message);
    return false;
  };
  if (array.isEmpty() || array.size() > 256) {
    return fail("gesture requires 1..256 finite steps");
  }
  for (const auto &value : array) {
    if (!value.isObject()) {
      return fail("gesture step is not an object");
    }
    const auto o = value.toObject();
    const auto op = o.value("op").toString();
    SeatEvent e{};
    QSet<QString> fields{QStringLiteral("op")};
    if (op == QLatin1String("key") || op == QLatin1String("button")) {
      input = true;
      e.kind = op == QLatin1String("key") ? SeatEvent::Key : SeatEvent::Button;
      fields.insert("pressed");
      if (!o.value("pressed").isBool()) {
        return fail("invalid gesture transition");
      }
      e.pressed = o.value("pressed").toBool();
      if (e.kind == SeatEvent::Key) {
        fields.insert("code");
        if (!integer(o.value("code"), 1, KEY_MAX)) {
          return fail("invalid gesture key");
        }
        e.code = o.value("code").toInt();
      } else {
        fields.insert("button");
        const auto b = o.value("button").toString();
        if (b == QLatin1String("left"))
          e.code = BTN_LEFT;
        else if (b == QLatin1String("right"))
          e.code = BTN_RIGHT;
        else if (b == QLatin1String("middle"))
          e.code = BTN_MIDDLE;
        else if (b == QLatin1String("back"))
          e.code = BTN_SIDE;
        else if (b == QLatin1String("forward"))
          e.code = BTN_EXTRA;
        else
          return fail("unsupported gesture button");
      }
      auto &held = e.kind == SeatEvent::Key ? keys : buttons;
      if (held.contains(e.code) == e.pressed) {
        return fail("unbalanced gesture transition");
      }
      if (e.pressed)
        held.insert(e.code);
      else
        held.remove(e.code);
      const bool ctrl = keys.contains(KEY_LEFTCTRL) ||
                        keys.contains(KEY_RIGHTCTRL),
                 alt =
                     keys.contains(KEY_LEFTALT) || keys.contains(KEY_RIGHTALT),
                 meta = keys.contains(KEY_LEFTMETA) ||
                        keys.contains(KEY_RIGHTMETA);
      if ((alt && keys.contains(KEY_F4)) ||
          (ctrl && (keys.contains(KEY_Q) || keys.contains(KEY_W))) ||
          (meta && keys.contains(KEY_Q))) {
        return fail("close/quit chord forbidden in exact-session gesture");
      }
    } else if (op == QLatin1String("motion") ||
               op == QLatin1String("relative_motion")) {
      input = true;
      const bool relative = op == QLatin1String("relative_motion");
      e.kind = relative ? SeatEvent::RelativeMotion : SeatEvent::Motion;
      const QString x = relative ? QStringLiteral("dx") : QStringLiteral("x"),
                    y = relative ? QStringLiteral("dy") : QStringLiteral("y");
      fields.insert(x);
      fields.insert(y);
      if (!o.value(x).isDouble() || !o.value(y).isDouble()) {
        return fail("invalid gesture motion");
      }
      e.position = QPointF(o.value(x).toDouble(), o.value(y).toDouble());
      if (!std::isfinite(e.position.x()) || !std::isfinite(e.position.y())) {
        return fail("nonfinite gesture motion");
      }
      if (relative) {
        if (std::abs(e.position.x()) > 4096 ||
            std::abs(e.position.y()) > 4096 || e.position.isNull())
          return fail("relative gesture delta outside bound");
      } else if (e.position.x() < 0 || e.position.y() < 0 ||
                 e.position.x() >= size.width() ||
                 e.position.y() >= size.height())
        return fail("gesture point outside original target");
    } else if (op == QLatin1String("scroll")) {
      input = true;
      e.kind = SeatEvent::Axis;
      fields.unite({"axis", "delta", "delta_v120", "source", "inverted"});
      const auto axis = o.value("axis").toString();
      if (axis != QLatin1String("vertical") &&
          axis != QLatin1String("horizontal"))
        return fail("invalid gesture scroll axis");
      e.axis =
          axis == QLatin1String("vertical") ? Qt::Vertical : Qt::Horizontal;
      if (!o.value("delta").isDouble() ||
          !std::isfinite(o.value("delta").toDouble()) ||
          std::abs(o.value("delta").toDouble()) > 1000000 ||
          !integer(o.value("delta_v120"), std::numeric_limits<qint32>::min(),
                   std::numeric_limits<qint32>::max()))
        return fail("invalid gesture scroll");
      e.delta = o.value("delta").toDouble();
      e.deltaV120 = o.value("delta_v120").toInt();
      e.source = o.value("source").toString();
      if (e.source != QLatin1String("wheel") &&
          e.source != QLatin1String("finger") &&
          e.source != QLatin1String("continuous"))
        return fail("invalid gesture scroll source");
      if (e.source != QLatin1String("wheel") && e.deltaV120 != 0)
        return fail("v120 scroll deltas require wheel source");
      if (o.contains("inverted") && !o.value("inverted").isBool())
        return fail("invalid gesture scroll direction");
      e.inverted = o.value("inverted").toBool();
    } else if (op == QLatin1String("wait") || op == QLatin1String("await")) {
      const bool wait = op == QLatin1String("wait");
      e.kind = wait ? SeatEvent::Delay : SeatEvent::Condition;
      const QString field =
          wait ? QStringLiteral("duration_ms") : QStringLiteral("timeout_ms");
      fields.insert(field);
      if (!integer(o.value(field), 1, 1500))
        return fail("gesture wait outside finite bound");
      e.durationMs = o.value(field).toInt();
      budget += e.durationMs;
      if (!wait) {
        fields.insert("condition");
        e.condition = o.value("condition").toString();
        if (e.condition != QLatin1String("bindings_ready") &&
            e.condition != QLatin1String("pointer_locked") &&
            e.condition != QLatin1String("client_unlocked"))
          return fail("unsupported gesture condition");
      }
    } else
      return fail("unsupported gesture opcode");
    for (auto it = o.begin(); it != o.end(); ++it) {
      if (!fields.contains(it.key()))
        return fail("unknown gesture step field");
    }
    decoded.push_back(e);
  }
  if (!input || !keys.isEmpty() || !buttons.isEmpty() || budget > 1500)
    return fail("gesture must balance all input and reserve at most 1500ms");
  *events = std::move(decoded);
  return true;
}

} // namespace KWin
