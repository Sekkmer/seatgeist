// SPDX-License-Identifier: MIT OR Apache-2.0
#include "eventbatch.h"
#include <QJsonArray>
#include <QtTest>

class EventBatchTest : public QObject
{
    Q_OBJECT
private Q_SLOTS:
    void wholeBatchValidation()
    {
        const QJsonObject down{{"op", "key"}, {"code", 30}, {"pressed", true}};
        const QJsonObject up{{"op", "key"}, {"code", 30}, {"pressed", false}};
        QJsonObject action{{"action", "events"}, {"protocol", 1},
                           {"expires_at_unix_ms", 1000}, {"events", QJsonArray{down, up}}};
        QVector<KWin::SeatEvent> events;
        QString error;
        QVERIFY(KWin::decodeSeatEvents(action, QSizeF(100, 100), &events, &error));
        QCOMPARE(events.size(), 2);
        for (const QJsonArray bad : {
                 QJsonArray{}, QJsonArray{down}, QJsonArray{up, down}, QJsonArray{down, down, up},
                 QJsonArray{down, up, QJsonObject{{"op", "unknown"}}},
                 QJsonArray{down, up, QJsonObject{{"op", "motion"}, {"x", 100}, {"y", 1}}},
                 QJsonArray{QJsonObject{{"op", "motion"}, {"x", -1}, {"y", 1}}},
                 QJsonArray{QJsonObject{{"op", "key"}, {"code", 30.5}, {"pressed", true}}},
                 QJsonArray{QJsonObject{{"op", "axis"}, {"axis", "vertical"}, {"delta", 1}, {"delta_v120", 0.5}}}}) {
            action["events"] = bad;
            events.resize(1);
            QVERIFY(!KWin::decodeSeatEvents(action, QSizeF(100, 100), &events, &error));
            QVERIFY(events.isEmpty()); // A valid prefix must never be delivered.
        }
        action["events"] = QJsonArray{down, up};
        action["protocol"] = 2;
        QVERIFY(!KWin::decodeSeatEvents(action, QSizeF(100, 100), &events, &error));
        action["protocol"] = 1;
        action.remove("expires_at_unix_ms");
        QVERIFY(!KWin::decodeSeatEvents(action, QSizeF(100, 100), &events, &error));
    }
};
QTEST_GUILESS_MAIN(EventBatchTest)
#include "eventbatch_test.moc"
