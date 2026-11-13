// SPDX-License-Identifier: MIT OR Apache-2.0
#include "gesturerunner.h"
#include <QtTest>
using namespace KWin;
class GestureRunnerTest : public QObject {
  Q_OBJECT
private Q_SLOTS:
  void timingAuthorizationAndEveryExit() {
    for (int scenario = 0; scenario < 7; ++scenario) {
      SeatEvent down{};
      down.kind = SeatEvent::Key;
      down.code = 17;
      down.pressed = true;
      SeatEvent wait{};
      wait.kind = SeatEvent::Delay;
      wait.durationMs = 200;
      SeatEvent up = down;
      up.pressed = false;
      bool valid = true, ok = false;
      int auth = 0, delivered = 0, finished = 0, releases = 0;
      GestureRunner::Hooks hooks;
      hooks.valid = [&] { return valid; };
      hooks.authorize = [&] { ++auth; };
      hooks.condition = [](const auto &) { return false; };
      hooks.deliver = [&](const auto &) {
        ++delivered;
        return scenario != 6;
      };
      hooks.release = [&](const auto &keys, const auto &buttons) {
        releases += keys.size() + buttons.size();
      };
      hooks.finished = [&](bool result, const QString &) {
        ok = result;
        ++finished;
      };
      {
        GestureRunner runner({down, wait, up}, std::move(hooks));
        runner.tick(0);
        QCOMPARE(delivered, 0);
        QCOMPARE(auth, 1);
        if (scenario == 1) {
          runner.authorized(false, 1);
        } else {
          runner.authorized(true, 1);
        }
        if (scenario == 0) {
          runner.tick(2);
          runner.authorized(true, 3);
          runner.tick(100);
          runner.authorized(true, 101);
          QCOMPARE(delivered, 1);
          runner.tick(203);
          QCOMPARE(delivered, 1);
          runner.authorized(true, 204);
          runner.tick(205);
          QVERIFY(ok);
        } else if (scenario == 2) {
          runner.cancel();
          runner.cancel();
          runner.authorized(true, 2);
        } else if (scenario == 3) {
          valid = false;
          runner.tick(2);
        } else if (scenario == 4) {
          runner.tick(2500);
          runner.authorized(true, 2501);
        }
        // scenario5 destructor balances a pending press; scenario6 delivery
        // refused.
      }
      QCOMPARE(finished, 1);
      QCOMPARE(ok, scenario == 0);
      QCOMPARE(releases, (scenario >= 2 && scenario <= 5) ? 1 : 0);
    }
  }
  void conditionCannotBeReplacedByDelayOrStaleAuthorization() {
    for (int scenario = 0; scenario < 4; ++scenario) {
      SeatEvent condition{};
      condition.kind = SeatEvent::Condition;
      condition.condition = "client_unlocked";
      condition.durationMs = 100;
      SeatEvent motion{};
      motion.kind = SeatEvent::Motion;
      motion.position = {3, 4};
      bool unlocked = false, ok = false;
      int delivered = 0, auth = 0;
      GestureRunner::Hooks hooks;
      hooks.valid = []() { return true; };
      hooks.authorize = [&] { ++auth; };
      hooks.condition = [&](const auto &) { return unlocked; };
      hooks.deliver = [&](const auto &) {
        ++delivered;
        return true;
      };
      hooks.release = [](const auto &, const auto &) {};
      hooks.finished = [&](bool v, const auto &) { ok = v; };
      GestureRunner runner({condition, motion}, std::move(hooks));
      runner.tick(0);
      runner.authorized(true, 1);
      runner.tick(20);
      QCOMPARE(delivered, 0);
      if (scenario == 0) {
        unlocked = true;
        runner.tick(21);
        QCOMPARE(delivered, 0);
        runner.authorized(true, 22);
        runner.tick(23);
        QVERIFY(ok);
      } else if (scenario == 1) {
        runner.tick(101);
        unlocked = true;
        runner.authorized(true, 102);
      } else if (scenario == 2) {
        unlocked = true;
        runner.tick(21);
        unlocked = false;
        runner.authorized(true, 22);
      } else {
        runner.cancel();
        unlocked = true;
        runner.authorized(true, 22);
      }
      QCOMPARE(delivered, scenario == 0 ? 1 : 0);
    }
  }
};
QTEST_GUILESS_MAIN(GestureRunnerTest)
#include "gesturerunner_test.moc"
