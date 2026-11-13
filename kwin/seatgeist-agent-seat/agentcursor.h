// SPDX-License-Identifier: MIT OR Apache-2.0
#pragma once
#include <QJsonObject>
#include <wayland/pointer.h>
namespace KWin {
// Metadata only: never sample a surface cursor image or infer its shape.
class AgentCursorState {
public:
  void reset() {
    ++m_revision;
    m_kind = QStringLiteral("unknown");
    m_name.clear();
  }
  void update(const PointerCursor &cursor) {
    ++m_revision;
    m_name.clear();
    if (const auto *name = std::get_if<QByteArray>(&cursor)) {
      m_kind = QStringLiteral("named");
      m_name = QString::fromUtf8(*name);
    } else {
      const auto *surface = std::get<PointerSurfaceCursor *>(cursor);
      m_kind = surface && surface->surface() ? QStringLiteral("surface")
                                             : QStringLiteral("hidden");
    }
  }
  QJsonObject snapshot(bool bound) const {
    return {{"revision", qint64(m_revision)},
            {"kind", bound ? m_kind : QStringLiteral("unknown")},
            {"name", bound && !m_name.isEmpty()
                         ? QJsonValue(m_name)
                         : QJsonValue(QJsonValue::Null)}};
  }

private:
  quint64 m_revision = 0;
  QString m_kind = QStringLiteral("unknown"), m_name;
};
} // namespace KWin
