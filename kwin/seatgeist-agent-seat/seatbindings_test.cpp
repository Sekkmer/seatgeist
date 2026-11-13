// SPDX-License-Identifier: MIT OR Apache-2.0
#include "agentcursor.h"
#include "altpointclick.h"
#include "gesturerunner.h"
#include "seatbindings.h"

#include "pointer-constraints-client.h"
#include "relative-pointer-client.h"
#include <wayland/clientconnection.h>
#include <wayland/compositor.h>
#include <wayland/display.h>
#include <wayland/keyboard.h>
#include <wayland/pointerconstraints_v1.h>
#include <wayland/relativepointer_v1.h>
#include <wayland/seat.h>
#include <wayland/surface.h>
#include <core/gpumanager.h>

#include <QCoreApplication>
#include <QTest>
#include <QTemporaryFile>
#include <QScopeGuard>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>
#include <wayland-client.h>
#include <xkbcommon/xkbcommon.h>

#include <cstring>
#include <map>
#include <vector>

using namespace KWin;

namespace {
// A real protocol client whose seat/device binding is controlled by the test.
struct Client {
  wl_display *display;
  wl_client *serverClient;
  wl_registry *registry;
  wl_compositor *compositor = nullptr;
  wl_surface *surface = nullptr;
  wl_shm *shm = nullptr;
  wl_buffer *buffer = nullptr;
  std::vector<wl_seat *> seats;
  std::vector<wl_keyboard *> keyboards;
  std::vector<wl_pointer *> pointers;
  std::vector<uint32_t> keys;
  int enters = 0;
  uint32_t pointerEnterSerial = 0;
  int motions = 0;
  int lockEvents = 0, unlockEvents = 0;
  bool unlockOnAlt = false;
  std::vector<uint32_t> releases, buttons;
  QPointF lastMotion;
  zwp_relative_pointer_manager_v1 *relativeManager = nullptr;
  zwp_pointer_constraints_v1 *constraints = nullptr;
  zwp_relative_pointer_v1 *relative = nullptr;
  zwp_locked_pointer_v1 *locked = nullptr;
  std::vector<QPointF> deltas, unaccelerated;

  explicit Client(KWin::Display &server) {
    int sockets[2];
    if (socketpair(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0, sockets) != 0) {
      qFatal("socketpair failed");
    }
    serverClient = server.createClient(sockets[0])->client();
    display = wl_display_connect_to_fd(sockets[1]);
    registry = wl_display_get_registry(display);
    static const wl_registry_listener listener{
        [](void *data, wl_registry *registry, uint32_t name,
           const char *interface, uint32_t version) {
          auto &self = *static_cast<Client *>(data);
          if (std::strcmp(interface, "wl_seat") == 0) {
            auto *seat = static_cast<wl_seat *>(wl_registry_bind(
                registry, name, &wl_seat_interface, std::min(version, 9u)));
            static const wl_seat_listener seatListener{
                [](void *, wl_seat *, uint32_t) {},
                [](void *, wl_seat *, const char *) {},
            };
            wl_seat_add_listener(seat, &seatListener, nullptr);
            self.seats.push_back(seat);
          } else if (std::strcmp(interface,
                                 "zwp_relative_pointer_manager_v1") == 0) {
            self.relativeManager =
                static_cast<zwp_relative_pointer_manager_v1 *>(wl_registry_bind(
                    registry, name, &zwp_relative_pointer_manager_v1_interface,
                    1));
          } else if (std::strcmp(interface, "zwp_pointer_constraints_v1") ==
                     0) {
            self.constraints =
                static_cast<zwp_pointer_constraints_v1 *>(wl_registry_bind(
                    registry, name, &zwp_pointer_constraints_v1_interface, 1));
          } else if (std::strcmp(interface, "wl_shm") == 0) {
            self.shm = static_cast<wl_shm *>(wl_registry_bind(registry, name, &wl_shm_interface, 1));
            static const wl_shm_listener listener{[](void *, wl_shm *, uint32_t) {}};
            wl_shm_add_listener(self.shm, &listener, nullptr);
          } else if (std::strcmp(interface, "wl_compositor") == 0) {
            self.compositor = static_cast<wl_compositor *>(
                wl_registry_bind(registry, name, &wl_compositor_interface, 1));
            self.surface = wl_compositor_create_surface(self.compositor);
          }
        },
        [](void *, wl_registry *, uint32_t) {},
    };
    wl_registry_add_listener(registry, &listener, this);
  }

  ~Client() {
    if (buffer) { wl_buffer_destroy(buffer); }
    if (shm) { wl_shm_destroy(shm); }
    if (locked) {
      zwp_locked_pointer_v1_destroy(locked);
    }
    if (relative) {
      zwp_relative_pointer_v1_destroy(relative);
    }
    if (constraints) {
      zwp_pointer_constraints_v1_destroy(constraints);
    }
    if (relativeManager) {
      zwp_relative_pointer_manager_v1_destroy(relativeManager);
    }
    for (auto *keyboard : keyboards) {
      wl_keyboard_destroy(keyboard);
    }
    for (auto *pointer : pointers) {
      wl_pointer_destroy(pointer);
    }
    for (auto *seat : seats) {
      wl_seat_destroy(seat);
    }
    if (surface) {
      wl_surface_destroy(surface);
    }
    if (compositor) {
      wl_compositor_destroy(compositor);
    }
    wl_registry_destroy(registry);
    wl_display_disconnect(display);
  }

  void pump(KWin::Display &server) {
    for (int i = 0; i < 12; ++i) {
      wl_display_flush(display);
      server.dispatchEvents();
      server.flush();
      pollfd fd{wl_display_get_fd(display), POLLIN, 0};
      if (poll(&fd, 1, 0) > 0) {
        if (wl_display_dispatch(display) < 0) {
          qFatal("Wayland client disconnected");
        }
      }
      wl_display_dispatch_pending(display);
    }
  }

  void bindPointer(size_t index) {
    auto *pointer = wl_seat_get_pointer(seats.at(index));
    static const wl_pointer_listener listener{
        .enter =
            [](void *data, wl_pointer *, uint32_t serial, wl_surface *,
               wl_fixed_t, wl_fixed_t) {
              static_cast<Client *>(data)->pointerEnterSerial = serial;
            },
        .leave = [](void *, wl_pointer *, uint32_t, wl_surface *) {},
        .motion =
            [](void *data, wl_pointer *, uint32_t, wl_fixed_t x, wl_fixed_t y) {
              auto &c = *static_cast<Client *>(data);
              ++c.motions;
              c.lastMotion =
                  QPointF(wl_fixed_to_double(x), wl_fixed_to_double(y));
            },
        .button =
            [](void *data, wl_pointer *, uint32_t, uint32_t, uint32_t,
               uint32_t state) {
              static_cast<Client *>(data)->buttons.push_back(state);
            },
        .axis = [](void *, wl_pointer *, uint32_t, uint32_t, wl_fixed_t) {},
        .frame = [](void *, wl_pointer *) {},
        .axis_source = [](void *, wl_pointer *, uint32_t) {},
        .axis_stop = [](void *, wl_pointer *, uint32_t, uint32_t) {},
        .axis_discrete = [](void *, wl_pointer *, uint32_t, int32_t) {},
        .axis_value120 = [](void *, wl_pointer *, uint32_t, int32_t) {},
        .axis_relative_direction = [](void *, wl_pointer *, uint32_t,
                                      uint32_t) {},
    };
    wl_pointer_add_listener(pointer, &listener, this);
    pointers.push_back(pointer);
  }

  void bindRelative(size_t index) {
    relative = zwp_relative_pointer_manager_v1_get_relative_pointer(
        relativeManager, pointers.at(index));
    static const zwp_relative_pointer_v1_listener listener{
        [](void *data, zwp_relative_pointer_v1 *, uint32_t, uint32_t,
           wl_fixed_t dx, wl_fixed_t dy, wl_fixed_t ux, wl_fixed_t uy) {
          auto &client = *static_cast<Client *>(data);
          client.deltas.emplace_back(wl_fixed_to_double(dx),
                                     wl_fixed_to_double(dy));
          client.unaccelerated.emplace_back(wl_fixed_to_double(ux),
                                            wl_fixed_to_double(uy));
        },
    };
    zwp_relative_pointer_v1_add_listener(relative, &listener, this);
  }

  void mapSurface() {
    QTemporaryFile file;
    if (!shm || !file.open() || !file.resize(100 * 80 * 4)) { qFatal("fixture shm setup failed"); }
    auto *pool = wl_shm_create_pool(shm, file.handle(), 100 * 80 * 4);
    buffer = wl_shm_pool_create_buffer(pool, 0, 100, 80, 400, WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);
    wl_surface_attach(surface, buffer, 0, 0);
    wl_surface_damage(surface, 0, 0, 100, 80);
    wl_surface_commit(surface);
  }

  void lock(size_t pointerIndex, wl_region *region = nullptr, uint32_t lifetime = ZWP_POINTER_CONSTRAINTS_V1_LIFETIME_PERSISTENT) {
    locked = zwp_pointer_constraints_v1_lock_pointer(
        constraints, surface, pointers.at(pointerIndex), region, lifetime);
    static const zwp_locked_pointer_v1_listener listener{
        [](void *data, zwp_locked_pointer_v1 *) { ++static_cast<Client *>(data)->lockEvents; },
        [](void *data, zwp_locked_pointer_v1 *) { ++static_cast<Client *>(data)->unlockEvents; }};
    zwp_locked_pointer_v1_add_listener(locked, &listener, this);
  }

  void bindKeyboard(size_t index) {
    auto *keyboard = wl_seat_get_keyboard(seats.at(index));
    static const wl_keyboard_listener listener{
        [](void *, wl_keyboard *, uint32_t, int32_t fd, uint32_t) {
          close(fd);
        },
        [](void *data, wl_keyboard *, uint32_t, wl_surface *, wl_array *) {
          ++static_cast<Client *>(data)->enters;
        },
        [](void *, wl_keyboard *, uint32_t, wl_surface *) {},
        [](void *data, wl_keyboard *, uint32_t, uint32_t, uint32_t key,
           uint32_t state) {
          auto &client = *static_cast<Client *>(data);
          if (state == WL_KEYBOARD_KEY_STATE_PRESSED) {
            client.keys.push_back(key);
            if (key == 56 && client.unlockOnAlt && client.locked) {
              zwp_locked_pointer_v1_destroy(client.locked);
              client.locked = nullptr;
            }
          } else {
            client.releases.push_back(key);
          }
        },
        [](void *, wl_keyboard *, uint32_t, uint32_t, uint32_t, uint32_t,
           uint32_t) {},
        [](void *, wl_keyboard *, int32_t, int32_t) {},
    };
    wl_keyboard_add_listener(keyboard, &listener, this);
    keyboards.push_back(keyboard);
  }
};

void keymap(SeatInterface &seat) {
  auto *context = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
  auto *map =
      xkb_keymap_new_from_names(context, nullptr, XKB_KEYMAP_COMPILE_NO_FLAGS);
  char *text = xkb_keymap_get_as_string(map, XKB_KEYMAP_FORMAT_TEXT_V1);
  seat.keyboard()->setKeymap(QByteArray(text));
  free(text);
  xkb_keymap_unref(map);
  xkb_context_unref(context);
}

void press(KWin::Display &display, SeatInterface &seat, uint32_t key) {
  seat.notifyKeyboardKey(key, KeyboardKeyState::Pressed, display.nextSerial());
  seat.notifyKeyboardKey(key, KeyboardKeyState::Released, display.nextSerial());
}
} // namespace

class SeatBindingsTest : public QObject {
  Q_OBJECT
private Q_SLOTS:
  void independentConstraintLifecycleUsesRealClientRequestAndCommittedRegion() {
    // KWin SHM buffers require its manager singleton even without rendering.
    // CTest gives this fixture a private /dev and no render-node enumeration.
    QVERIFY(!QFile::exists(QStringLiteral("/dev/udmabuf")));
    QVERIFY(qEnvironmentVariableIsSet("KWIN_RENDER_NODES") && qEnvironmentVariableIsEmpty("KWIN_RENDER_NODES"));
    GpuManager::s_self = std::make_unique<GpuManager>();
    const auto resetManager = qScopeGuard([] { GpuManager::s_self.reset(); });
    KWin::Display display;
    display.createShm();
    CompositorInterface compositor(&display);
    PointerConstraintsV1Interface constraints(&display);
    RelativePointerManagerV1Interface manager(&display);
    std::vector<SurfaceInterface *> surfaces;
    connect(&compositor, &CompositorInterface::surfaceCreated, this, [&](auto *s) { surfaces.push_back(s); });
    SeatInterface primary(&display, QStringLiteral("primary")), agent(&display, QStringLiteral("agent"));
    primary.setHasPointer(true);
    agent.setHasPointer(true);
    agent.setHasKeyboard(true);
    keymap(agent);
    SeatBindings bindings(display, &primary);
    bindings.watch(&agent);
    Client client(display), other(display);
    client.pump(display);
    other.pump(display);
    client.mapSurface();
    other.mapSurface();
    client.bindPointer(1); // pointers[0] is the agent, pointers[1] is primary.
    client.bindPointer(0);
    client.bindKeyboard(1);
    client.bindRelative(0);
    client.pump(display);
    other.pump(display);
    auto *surface = surfaces.at(0);
    primary.notifyPointerEnter(surfaces.at(1), QPointF(12, 13));
    agent.setFocusedKeyboardSurface(surface);
    agent.notifyPointerEnter(surface, QPointF(30, 40));
    const auto physical = primary.pointerPos();
    auto *physicalFocus = primary.focusedPointerSurface();

    // A primary-pointer request must never become an agent lock.
    bindings.setConstraintTarget(&agent, surface);
    client.lock(1);
    client.pump(display);
    QVERIFY(!bindings.lockedPointerRequested(&agent, client.serverClient, surface));
    QVERIFY(!surface->lockedPointer()->isLocked());
    zwp_locked_pointer_v1_destroy(client.locked);
    client.locked = nullptr;
    client.pump(display);
    bindings.setConstraintTarget(&agent, nullptr);

    client.lock(0);
    client.pump(display);
    QVERIFY(bindings.lockedPointerRequested(&agent, client.serverClient, surface));
    QVERIFY(bindings.pointerLockRegionContains(&agent, surface));
    QVERIFY(!surface->lockedPointer()->isLocked()); // no authorized target
    bindings.setConstraintTarget(&agent, surface);
    client.pump(display);
    QVERIFY(bindings.lockedPointerReady(&agent, client.serverClient, surface));
    QCOMPARE(client.lockEvents, 1); // No test-side setLocked(true).
    QVERIFY(bindings.deliverRelativePointer(&agent, client.serverClient, surface, QPointF(8, -3), std::chrono::microseconds(1000)));
    client.pump(display);
    QCOMPARE(client.deltas.size(), size_t(1));
    QCOMPARE(client.deltas.front(), QPointF(8, -3));

    auto *outside = wl_compositor_create_region(client.compositor);
    wl_region_add(outside, 60, 60, 10, 10);
    zwp_locked_pointer_v1_set_region(client.locked, outside);
    client.pump(display);
    QVERIFY(surface->lockedPointer()->isLocked()); // set_region is double buffered
    wl_surface_commit(client.surface);
    client.pump(display);
    QVERIFY(!bindings.pointerLockRegionContains(&agent, surface));
    QVERIFY(!surface->lockedPointer()->isLocked());
    QCOMPARE(client.unlockEvents, 1);
    zwp_locked_pointer_v1_set_region(client.locked, nullptr);
    wl_surface_commit(client.surface);
    client.pump(display);
    QVERIFY(surface->lockedPointer()->isLocked());
    QCOMPARE(client.lockEvents, 2);
    wl_region_destroy(outside);

    agent.notifyPointerLeave();
    client.pump(display);
    QVERIFY(!surface->lockedPointer()->isLocked());
    agent.notifyPointerEnter(surface, QPointF(30, 40));
    client.pump(display);
    QVERIFY(surface->lockedPointer()->isLocked());
    // A physical pointer on this surface suspends our activation. No primary
    // position/focus changes are made by the lifecycle itself.
    primary.notifyPointerEnter(surface, physical);
    client.pump(display);
    QVERIFY(!surface->lockedPointer()->isLocked());
    primary.notifyPointerEnter(physicalFocus, physical);
    client.pump(display);
    QVERIFY(surface->lockedPointer()->isLocked());
    auto release = bindings.observeActiveLock(&agent, client.serverClient, surface);
    QVERIFY(release);
    zwp_locked_pointer_v1_destroy(client.locked);
    client.locked = nullptr;
    client.pump(display);
    QVERIFY(release->clientDestroyed && !release->lock);
    QVERIFY(!bindings.lockedPointerRequested(&agent, client.serverClient, surface));
    client.lock(0, nullptr, ZWP_POINTER_CONSTRAINTS_V1_LIFETIME_ONESHOT);
    client.pump(display);
    QVERIFY(surface->lockedPointer()->isLocked());
    bindings.setConstraintTarget(&agent, nullptr); // lane release / owner expiry
    client.pump(display);
    QVERIFY(!surface->lockedPointer());
    QVERIFY(!bindings.lockedPointerRequested(&agent, client.serverClient, surface));
    zwp_locked_pointer_v1_destroy(client.locked);
    client.locked = nullptr;
    client.pump(display);
    bindings.setConstraintTarget(&agent, surface);
    client.lock(0);
    client.pump(display);
    QVERIFY(surface->lockedPointer()->isLocked());
    QPointer<SurfaceInterface> destroyed = surface;
    wl_surface_destroy(client.surface);
    client.surface = nullptr;
    client.pump(display);
    QCoreApplication::processEvents();
    QVERIFY(!destroyed);
    QCOMPARE(primary.pointerPos(), physical);
    QCOMPARE(primary.focusedPointerSurface(), physicalFocus);
    bindings.forget(&agent);
  }

  void cursorMetadataTracksActualClientHideWithoutPhysicalCursorOrPixels() {
    KWin::Display display;
    CompositorInterface compositor(&display);
    SurfaceInterface *surface = nullptr;
    connect(&compositor, &CompositorInterface::surfaceCreated, this,
            [&](auto *s) { surface = s; });
    SeatInterface primary(&display, QStringLiteral("primary")),
        agent(&display, QStringLiteral("agent"));
    primary.setHasPointer(true);
    agent.setHasPointer(true);
    AgentCursorState cursor;
    connect(agent.pointer(), &PointerInterface::cursorChanged, this,
            [&](const PointerCursor &value) { cursor.update(value); });
    QCOMPARE(cursor.snapshot(true)["kind"].toString(),
             QStringLiteral("unknown"));
    Client client(display);
    client.pump(display);
    client.bindPointer(1);
    client.pump(display);
    agent.notifyPointerEnter(surface, QPointF(10, 10));
    agent.notifyPointerFrame();
    client.pump(display);
    QVERIFY(client.pointerEnterSerial != 0);
    const auto physical = primary.pointerPos();
    auto *cursorSurface = wl_compositor_create_surface(client.compositor);
    client.pump(display);
    wl_pointer_set_cursor(client.pointers[0], client.pointerEnterSerial,
                          cursorSurface, 0, 0);
    client.pump(display);
    QCOMPARE(cursor.snapshot(true)["kind"].toString(),
             QStringLiteral("surface"));
    const auto before = cursor.snapshot(true)["revision"].toInteger();
    wl_pointer_set_cursor(client.pointers[0], client.pointerEnterSerial,
                          nullptr, 0, 0);
    client.pump(display);
    QCOMPARE(cursor.snapshot(true)["kind"].toString(),
             QStringLiteral("hidden"));
    QVERIFY(cursor.snapshot(true)["revision"].toInteger() > before);
    QCOMPARE(primary.pointerPos(), physical);
    wl_surface_destroy(cursorSurface);
    client.pump(display);
    // Named shape metadata uses the same production signal handler; it is not a
    // claim that a surface cursor's pixels establish an I-beam shape.
    agent.pointer()->cursorChanged(QByteArray("text"));
    QCOMPARE(cursor.snapshot(true)["name"].toString(), QStringLiteral("text"));
    QCOMPARE(cursor.snapshot(false)["kind"].toString(),
             QStringLiteral("unknown"));
    QVERIFY(cursor.snapshot(false)["name"].isNull());
    cursor.reset();
    QCOMPARE(cursor.snapshot(true)["kind"].toString(),
             QStringLiteral("unknown"));
  }

  void finiteGestureUsesActualClientUnlockAndRelativeProtocol() {
    for (int scenario = 0; scenario < 3; ++scenario) {
      KWin::Display display;
      CompositorInterface compositor(&display);
      PointerConstraintsV1Interface constraints(&display);
      RelativePointerManagerV1Interface manager(&display);
      SurfaceInterface *surface = nullptr;
      connect(&compositor, &CompositorInterface::surfaceCreated, this,
              [&](auto *s) { surface = s; });
      SeatInterface primary(&display, QStringLiteral("primary")),
          agent(&display, QStringLiteral("agent"));
      primary.setHasPointer(true);
      agent.setHasPointer(true);
      agent.setHasKeyboard(true);
      keymap(agent);
      SeatBindings bindings(display);
      bindings.watch(&agent);
      Client client(display);
      client.pump(display);
      client.bindPointer(1);
      client.bindKeyboard(1);
      client.pump(display);
      agent.setFocusedKeyboardSurface(surface);
      agent.notifyPointerEnter(surface, QPointF(10, 10));
      client.bindRelative(0);
      client.lock(0);
      client.pump(display);
      surface->lockedPointer()->setLocked(true);
      const auto physical = primary.pointerPos();
      const auto release =
          bindings.observeActiveLock(&agent, client.serverClient, surface);
      QVERIFY(release);
      client.unlockOnAlt = scenario == 0;
      QVector<SeatEvent> steps;
      SeatEvent down{};
      down.kind = SeatEvent::Key;
      down.code = scenario == 2 ? 17 : 56;
      down.pressed = true;
      SeatEvent up = down;
      up.pressed = false;
      steps.push_back(down);
      if (scenario == 2) {
        SeatEvent relative{};
        relative.kind = SeatEvent::RelativeMotion;
        relative.position = {8, -3};
        steps.push_back(relative);
        SeatEvent wait{};
        wait.kind = SeatEvent::Delay;
        wait.durationMs = 20;
        steps.push_back(wait);
        steps.push_back(relative);
      } else {
        SeatEvent condition{};
        condition.kind = SeatEvent::Condition;
        condition.condition = "client_unlocked";
        condition.durationMs = 100;
        steps.push_back(condition);
        SeatEvent motion{};
        motion.kind = SeatEvent::Motion;
        motion.position = {196, 116};
        steps.push_back(motion);
        SeatEvent button{};
        button.kind = SeatEvent::Button;
        button.code = 272;
        button.pressed = true;
        steps.push_back(button);
        button.pressed = false;
        steps.push_back(button);
      }
      steps.push_back(up);
      bool auth = false, ok = false;
      int completions = 0;
      GestureRunner::Hooks hooks;
      hooks.valid = []() { return true; };
      hooks.authorize = [&] { auth = true; };
      hooks.condition = [&](const auto &) {
        return release->clientDestroyed && !release->lock &&
               !surface->lockedPointer();
      };
      hooks.deliver = [&](const SeatEvent &e) {
        if (e.kind == SeatEvent::Key)
          agent.notifyKeyboardKey(e.code,
                                  e.pressed ? KeyboardKeyState::Pressed
                                            : KeyboardKeyState::Released,
                                  display.nextSerial());
        else if (e.kind == SeatEvent::RelativeMotion)
          return bindings.deliverRelativePointer(&agent, client.serverClient,
                                                 surface, e.position,
                                                 std::chrono::microseconds(1));
        else if (e.kind == SeatEvent::Motion)
          agent.notifyPointerMotion(e.position);
        else if (e.kind == SeatEvent::Button)
          agent.notifyPointerButton(e.code, e.pressed
                                                ? PointerButtonState::Pressed
                                                : PointerButtonState::Released);
        agent.notifyPointerFrame();
        return true;
      };
      hooks.release = [&](const auto &keys, const auto &buttons) {
        for (auto key : keys)
          agent.notifyKeyboardKey(key, KeyboardKeyState::Released,
                                  display.nextSerial());
        for (auto button : buttons)
          agent.notifyPointerButton(button, PointerButtonState::Released);
        agent.notifyPointerFrame();
      };
      hooks.finished = [&](bool result, const auto &) {
        ok = result;
        ++completions;
      };
      {
        GestureRunner runner(steps, std::move(hooks));
        for (qint64 now = 0; now < 200 && !completions; ++now) {
          runner.tick(now);
          if (auth) {
            auth = false;
            runner.authorized(true, now);
          }
          if (scenario == 1 && surface->lockedPointer())
            surface->lockedPointer()->setLocked(false);
          client.pump(display);
        }
      }
      QCOMPARE(completions, 1);
      QCOMPARE(ok, scenario != 1);
      QCOMPARE(primary.pointerPos(), physical);
      QCOMPARE(client.keys,
               std::vector<uint32_t>{uint32_t(scenario == 2 ? 17 : 56)});
      QCOMPARE(client.releases, client.keys);
      if (scenario == 2) {
        QCOMPARE(client.deltas,
                 (std::vector<QPointF>{QPointF(8, -3), QPointF(8, -3)}));
        QCOMPARE(client.unaccelerated, client.deltas);
        QVERIFY(client.buttons.empty());
      } else {
        QCOMPARE(client.buttons.size(), scenario == 0 ? size_t(2) : size_t(0));
        if (scenario == 0)
          QCOMPARE(client.lastMotion, QPointF(196, 116));
      }
    }
  }

  void altPointClickWaitsForActualUnlockAndBalancesEveryExit() {
    for (int scenario = 0; scenario < 11; ++scenario) {
      KWin::Display display;
      CompositorInterface compositor(&display);
      PointerConstraintsV1Interface constraints(&display);
      SurfaceInterface *surface = nullptr;
      connect(&compositor, &CompositorInterface::surfaceCreated, this,
              [&](auto *s) { surface = s; });
      SeatInterface primary(&display, QStringLiteral("primary"));
      SeatInterface agent(&display, QStringLiteral("agent"));
      primary.setHasPointer(true);
      agent.setHasPointer(true);
      agent.setHasKeyboard(true);
      keymap(agent);
      SeatBindings bindings(display);
      bindings.watch(&agent);
      Client client(display);
      client.pump(display);
      client.bindPointer(1);
      client.bindKeyboard(1);
      client.pump(display);
      agent.setFocusedKeyboardSurface(surface);
      agent.notifyPointerEnter(surface, QPointF(10, 10));
      client.lock(0);
      client.pump(display);
      if (scenario != 8) {
        surface->lockedPointer()->setLocked(true);
      }
      const auto physical = primary.pointerPos();
      bool targetValid = true;
      int downs = 0, ups = 0, authorizations = 0, clicks = 0, completions = 0;
      bool success = false;
      client.unlockOnAlt =
          scenario == 0 || scenario == 4 || scenario == 5 || scenario == 6;
      AltPointClick::Hooks hooks;
      hooks.valid = [&] { return targetValid; };
      hooks.alt = [&](bool down) {
        if (down)
          ++downs;
        else
          ++ups;
        agent.notifyKeyboardKey(
            56, down ? KeyboardKeyState::Pressed : KeyboardKeyState::Released,
            display.nextSerial());
      };
      hooks.authorize = [&] { ++authorizations; };
      hooks.click = [&] {
        ++clicks;
        agent.notifyPointerMotion(QPointF(196, 116));
        agent.notifyPointerButton(0x110, PointerButtonState::Pressed);
        agent.notifyPointerButton(0x110, PointerButtonState::Released);
        agent.notifyPointerFrame();
        return true;
      };
      hooks.finished = [&](bool ok, const QString &) {
        ++completions;
        success = ok;
      };
      {
        AltPointClick gesture(bindings, &agent, surface, std::move(hooks));
        gesture.start(0);
        gesture.tick(1);
        QCOMPARE(clicks, 0);
        QCOMPARE(authorizations, 0); // cannot service unlock synchronously
        client.pump(display); // actual keyboard callback sends lock destroy;
                              // server processes it
        if (scenario == 0) {
          QVERIFY(!surface->lockedPointer());
          gesture.tick(2);
          QCOMPARE(authorizations, 1);
          QCOMPARE(clicks, 0); // wait for async daemon revalidation
          gesture.authorized(true, 3);
          QVERIFY(success);
        } else if (scenario == 1) {
          gesture.tick(1000);
        } // no client unlock
        else if (scenario == 2) {
          targetValid = false;
          gesture.tick(2);
        } else if (scenario == 3) {
          gesture.cancel();
          gesture.cancel();
          gesture.authorized(true, 3);
        } else if (scenario == 4) {
          gesture.tick(2);
          gesture.authorized(false, 3);
        } else if (scenario == 5) {
          gesture.tick(2);
          gesture.tick(1000);
          gesture.authorized(true, 1001);
        } else if (scenario == 6) {
          gesture.tick(2);
          targetValid = false;
          gesture.authorized(true, 3);
        } else if (scenario == 7) {
          surface->lockedPointer()->setLocked(false);
          gesture.tick(2);
          QCOMPARE(authorizations, 0);
          gesture.tick(1000); // server-only unlock is not client confirmation
        } else if (scenario == 9) {
          wl_surface_destroy(client.surface);
          client.surface = nullptr;
          client.pump(display);
          gesture.tick(2);
        }
        // scenario8 refuses before Alt down; scenario10 destructor releases a
        // pending Alt.
      }
      client.pump(display);
      QCOMPARE(completions, 1);
      QCOMPARE(downs, scenario == 8 ? 0 : 1);
      QCOMPARE(ups, downs);
      QCOMPARE(clicks, scenario == 0 ? 1 : 0);
      QCOMPARE(client.buttons.size(), scenario == 0 ? size_t(2) : size_t(0));
      if (scenario == 0) {
        QCOMPARE(client.lastMotion, QPointF(196, 116));
        QCOMPARE(client.keys, std::vector<uint32_t>{56});
        QCOMPARE(client.releases, std::vector<uint32_t>{56});
      }
      QCOMPARE(primary.pointerPos(), physical);
    }
  }

  void relativeDeliveryRequiresExactLiveBindingsAndLock() {
    KWin::Display display;
    CompositorInterface compositor(&display);
    RelativePointerManagerV1Interface relativeManager(&display);
    PointerConstraintsV1Interface constraints(&display);
    std::vector<SurfaceInterface *> surfaces;
    connect(&compositor, &CompositorInterface::surfaceCreated, this,
            [&](auto *surface) { surfaces.push_back(surface); });
    SeatInterface primary(&display, QStringLiteral("primary"));
    SeatInterface agent(&display, QStringLiteral("agent"));
    primary.setHasPointer(true);
    agent.setHasPointer(true);
    SeatBindings bindings(display);
    bindings.watch(&agent);
    Client client(display), other(display);
    client.pump(display);
    other.pump(display);
    QCOMPARE(surfaces.size(), size_t(2));
    auto *surface = surfaces[0];
    client.bindPointer(0);
    client.bindPointer(1);
    client.pump(display);
    primary.notifyPointerEnter(surfaces[1], QPointF(12, 13));
    agent.notifyPointerEnter(surface, QPointF(30, 40));
    client.pump(display);
    const auto physicalPosition = primary.pointerPos();
    const auto agentPosition = agent.pointerPos();
    const auto time = std::chrono::microseconds(1000000);
    QVERIFY(!bindings.deliverRelativePointer(&agent, client.serverClient,
                                             surface, QPointF(8, -3), time));
    client.bindRelative(0);
    client.lock(0);
    client.pump(display);
    surface->lockedPointer()->setLocked(true);
    // A primary-seat relative subscription/active lock is insufficient.
    QVERIFY(!bindings.relativePointerReady(&agent, client.serverClient));
    QVERIFY(!bindings.lockedPointerReady(&agent, client.serverClient, surface));
    QVERIFY(!bindings.deliverRelativePointer(&agent, client.serverClient,
                                             surface, QPointF(8, -3), time));
    zwp_locked_pointer_v1_destroy(client.locked);
    client.locked = nullptr;
    zwp_relative_pointer_v1_destroy(client.relative);
    client.relative = nullptr;
    client.pump(display);
    client.bindRelative(1);
    client.lock(1);
    client.pump(display);
    QVERIFY(bindings.relativePointerReady(&agent, client.serverClient));
    QVERIFY(!bindings.relativePointerReady(&agent, other.serverClient));
    QVERIFY(!bindings.lockedPointerReady(&agent, client.serverClient,
                                         surface)); // requested, inactive
    QVERIFY(!bindings.deliverRelativePointer(&agent, client.serverClient,
                                             surface, QPointF(8, -3), time));
    surface->lockedPointer()->setLocked(true);
    QVERIFY(bindings.lockedPointerReady(&agent, client.serverClient, surface));
    QVERIFY(!bindings.deliverRelativePointer(&agent, other.serverClient,
                                             surface, QPointF(8, -3), time));
    QVERIFY(!bindings.deliverRelativePointer(
        &agent, client.serverClient, surfaces[1], QPointF(8, -3), time));
    QVERIFY(!bindings.deliverRelativePointer(&agent, client.serverClient,
                                             surface, QPointF(4097, 0), time));
    QVERIFY(!bindings.deliverRelativePointer(&agent, client.serverClient,
                                             surface, QPointF(0, 0), time));
    client.pump(display);
    QVERIFY(client.deltas.empty());
    const int before = client.motions;
    agent.notifyPointerMotion(QPointF(80, 90));
    client.pump(display);
    QCOMPARE(client.motions, before); // actual KWin locked absolute suppression
    QVERIFY(client.deltas.empty());   // absolute motion is not relative motion
    const auto stableAgentPosition = agent.pointerPos();
    QVERIFY(bindings.deliverRelativePointer(&agent, client.serverClient,
                                            surface, QPointF(8, -3), time));
    client.pump(display);
    QCOMPARE(client.deltas, std::vector<QPointF>{QPointF(8, -3)});
    QCOMPARE(client.unaccelerated, client.deltas);
    QCOMPARE(primary.pointerPos(), physicalPosition);
    QCOMPARE(agent.pointerPos(), stableAgentPosition); // no absolute warp
    zwp_relative_pointer_v1_destroy(client.relative);
    client.relative = nullptr;
    client.pump(display);
    QVERIFY(!bindings.relativePointerReady(&agent, client.serverClient));
    QVERIFY(!bindings.deliverRelativePointer(&agent, client.serverClient,
                                             surface, QPointF(8, -3), time));
    client.bindRelative(1); // reused protocol IDs must require new live binding
    QVERIFY(!bindings.relativePointerReady(&agent, client.serverClient));
    client.pump(display);
    QVERIFY(bindings.relativePointerReady(&agent, client.serverClient));
    surface->lockedPointer()->setLocked(false);
    QVERIFY(!bindings.deliverRelativePointer(&agent, client.serverClient,
                                             surface, QPointF(8, -3), time));
    surface->lockedPointer()->setLocked(true);
    zwp_locked_pointer_v1_destroy(client.locked);
    client.locked = nullptr;
    client.pump(display);
    QVERIFY(!bindings.lockedPointerReady(&agent, client.serverClient, surface));
    client.lock(1);
    client.pump(display);
    surface->lockedPointer()->setLocked(true);
    QVERIFY(bindings.lockedPointerReady(&agent, client.serverClient, surface));
    wl_pointer_release(client.pointers.back());
    client.pointers.pop_back();
    client.pump(display);
    QVERIFY(!bindings.relativePointerReady(&agent, client.serverClient));
    QVERIFY(!bindings.lockedPointerReady(&agent, client.serverClient, surface));
    client.pump(display);
    QCOMPARE(client.deltas.size(), size_t(1));
    bindings.forget(&agent);
    QVERIFY(!bindings.relativePointerReady(&agent, client.serverClient));
    Q_UNUSED(agentPosition)
  }

  void firstKeyWaitsForActualBinding() {
    KWin::Display display;
    CompositorInterface compositor(&display);
    SurfaceInterface *surface = nullptr;
    connect(&compositor, &CompositorInterface::surfaceCreated, this,
            [&](auto *created) { surface = created; });
    SeatBindings bindings(display);
    QVERIFY(bindings.available());
    Client client(display);
    client.pump(display);
    QVERIFY(surface);
    SeatInterface seat(&display, QStringLiteral("agent"));
    seat.setHasKeyboard(true);
    seat.setHasPointer(true);
    keymap(seat);
    bindings.watch(&seat);

    // Reproduce the old bug: compositor focus alone silently loses keys.
    seat.setFocusedKeyboardSurface(surface);
    press(display, seat, 8);
    client.pump(display);
    QCOMPARE(client.seats.size(), size_t(1));
    QVERIFY(client.keys.empty());
    QVERIFY(!bindings.keyboardReady(&seat, client.serverClient));

    client.bindKeyboard(0);
    QVERIFY(!bindings.keyboardReady(&seat, client.serverClient));
    client.pump(display);
    QVERIFY(bindings.keyboardReady(&seat, client.serverClient));
    QCOMPARE(client.enters, 1);
    press(display, seat, 8);
    client.pump(display);
    QCOMPARE(client.keys, std::vector<uint32_t>{8});

    wl_keyboard_release(client.keyboards.back());
    client.keyboards.pop_back();
    client.pump(display);
    QVERIFY(!bindings.keyboardReady(&seat, client.serverClient));
    client.bindKeyboard(0); // Object-id reuse must require a fresh binding.
    QVERIFY(!bindings.keyboardReady(&seat, client.serverClient));
    client.pump(display);
    QVERIFY(bindings.keyboardReady(&seat, client.serverClient));
    bindings.forget(&seat);
    QVERIFY(!bindings.keyboardReady(&seat, client.serverClient));
  }

  void bindingsAreClientAndSeatSpecific() {
    KWin::Display display;
    SeatInterface primary(&display, QStringLiteral("primary"));
    SeatInterface agent(&display, QStringLiteral("agent"));
    primary.setHasKeyboard(true);
    agent.setHasKeyboard(true);
    agent.setHasPointer(true);
    keymap(primary);
    keymap(agent);
    SeatBindings bindings(display);
    bindings.watch(&agent);
    Client client(display);
    Client other(display);
    client.pump(display);
    other.pump(display);
    client.bindKeyboard(0);
    client.pump(display);
    QVERIFY(!bindings.keyboardReady(&agent, client.serverClient));
    other.bindKeyboard(1);
    other.pump(display);
    QVERIFY(bindings.keyboardReady(&agent, other.serverClient));
    QVERIFY(!bindings.keyboardReady(&agent, client.serverClient));
    client.bindKeyboard(1);
    client.pump(display);
    QVERIFY(bindings.keyboardReady(&agent, client.serverClient));
    QVERIFY(!bindings.pointerReady(&agent, client.serverClient));
    client.pointers.push_back(wl_seat_get_pointer(client.seats.at(1)));
    client.pump(display);
    QVERIFY(bindings.pointerReady(&agent, client.serverClient));
    QVERIFY(!bindings.pointerReady(&agent, other.serverClient));
    wl_pointer_release(client.pointers.back());
    client.pointers.pop_back();
    client.pump(display);
    QVERIFY(!bindings.pointerReady(&agent, client.serverClient));
  }
};

QTEST_GUILESS_MAIN(SeatBindingsTest)
#include "seatbindings_test.moc"
