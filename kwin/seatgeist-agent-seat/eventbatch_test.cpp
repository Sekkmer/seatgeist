// SPDX-License-Identifier: MIT OR Apache-2.0
#include "eventbatch.h"
#include <QJsonArray>
#include <QtTest>

class EventBatchTest : public QObject
{
    Q_OBJECT
private Q_SLOTS:
    void stagedAltPointClickIsStandalone()
    {
        QJsonObject click{{"op","alt_point_click"},{"x",9},{"y",8}};
        QJsonObject action{{"action","events"},{"protocol",1},{"expires_at_unix_ms",1000},{"events",QJsonArray{click}}};
        QVector<KWin::SeatEvent> events;QString error;
        QVERIFY(KWin::decodeSeatEvents(action,QSizeF(100,100),&events,&error));QCOMPARE(events[0].kind,KWin::SeatEvent::AltPointClick);
        action["events"]=QJsonArray{click,click};QVERIFY(!KWin::decodeSeatEvents(action,QSizeF(100,100),&events,&error));QVERIFY(events.empty());
        click["x"]=100;action["events"]=QJsonArray{click};QVERIFY(!KWin::decodeSeatEvents(action,QSizeF(100,100),&events,&error));
    }
    void finiteRelativeMotionIsStandalone()
    {
        QJsonObject relative{{"op","relative_motion"},{"dx",8.5},{"dy",-3}};
        QJsonObject action{{"action","events"},{"protocol",1},{"expires_at_unix_ms",1000},{"events",QJsonArray{relative}}};
        QVector<KWin::SeatEvent> events; QString error;
        QVERIFY(KWin::decodeSeatEvents(action, QSizeF(10,10), &events, &error));
        QCOMPARE(events.size(), 1); QCOMPARE(events[0].kind, KWin::SeatEvent::RelativeMotion);
        QCOMPARE(events[0].position, QPointF(8.5,-3));
        for (const QJsonArray bad : { QJsonArray{relative,relative},
                 QJsonArray{relative,QJsonObject{{"op","motion"},{"x",1},{"y",1}}},
                 QJsonArray{QJsonObject{{"op","relative_motion"},{"dx",4097},{"dy",0}}},
                 QJsonArray{QJsonObject{{"op","relative_motion"},{"dx",0},{"dy",0}}},
                 QJsonArray{QJsonObject{{"op","relative_motion"},{"dx","8"},{"dy",0}}} }) {
            action["events"] = bad;
            QVERIFY(!KWin::decodeSeatEvents(action, QSizeF(10,10), &events, &error)); QVERIFY(events.isEmpty());
        }
    }
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
