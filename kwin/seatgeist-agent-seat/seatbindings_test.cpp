// SPDX-License-Identifier: MIT OR Apache-2.0
#include "seatbindings.h"

#include <wayland/clientconnection.h>
#include <wayland/compositor.h>
#include <wayland/display.h>
#include <wayland/keyboard.h>
#include <wayland/seat.h>
#include <wayland/surface.h>

#include <QCoreApplication>
#include <QTest>
#include <wayland-client.h>
#include <xkbcommon/xkbcommon.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>

#include <cstring>
#include <map>
#include <vector>

using namespace KWin;

namespace
{
// A real protocol client whose seat/device binding is controlled by the test.
struct Client {
    wl_display *display;
    wl_client *serverClient;
    wl_registry *registry;
    wl_compositor *compositor = nullptr;
    wl_surface *surface = nullptr;
    std::vector<wl_seat *> seats;
    std::vector<wl_keyboard *> keyboards;
    std::vector<wl_pointer *> pointers;
    std::vector<uint32_t> keys;
    int enters = 0;

    explicit Client(KWin::Display &server)
    {
        int sockets[2];
        if (socketpair(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0, sockets) != 0) {
            qFatal("socketpair failed");
        }
        serverClient = server.createClient(sockets[0])->client();
        display = wl_display_connect_to_fd(sockets[1]);
        registry = wl_display_get_registry(display);
        static const wl_registry_listener listener{
            [](void *data, wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
                auto &self = *static_cast<Client *>(data);
                if (std::strcmp(interface, "wl_seat") == 0) {
                    auto *seat = static_cast<wl_seat *>(wl_registry_bind(registry, name, &wl_seat_interface, std::min(version, 9u)));
                    static const wl_seat_listener seatListener{
                        [](void *, wl_seat *, uint32_t) {},
                        [](void *, wl_seat *, const char *) {},
                    };
                    wl_seat_add_listener(seat, &seatListener, nullptr);
                    self.seats.push_back(seat);
                } else if (std::strcmp(interface, "wl_compositor") == 0) {
                    self.compositor = static_cast<wl_compositor *>(wl_registry_bind(registry, name, &wl_compositor_interface, 1));
                    self.surface = wl_compositor_create_surface(self.compositor);
                }
            },
            [](void *, wl_registry *, uint32_t) {},
        };
        wl_registry_add_listener(registry, &listener, this);
    }

    ~Client()
    {
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

    void pump(KWin::Display &server)
    {
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

    void bindKeyboard(size_t index)
    {
        auto *keyboard = wl_seat_get_keyboard(seats.at(index));
        static const wl_keyboard_listener listener{
            [](void *, wl_keyboard *, uint32_t, int32_t fd, uint32_t) { close(fd); },
            [](void *data, wl_keyboard *, uint32_t, wl_surface *, wl_array *) { ++static_cast<Client *>(data)->enters; },
            [](void *, wl_keyboard *, uint32_t, wl_surface *) {},
            [](void *data, wl_keyboard *, uint32_t, uint32_t, uint32_t key, uint32_t state) {
                if (state == WL_KEYBOARD_KEY_STATE_PRESSED) {
                    static_cast<Client *>(data)->keys.push_back(key);
                }
            },
            [](void *, wl_keyboard *, uint32_t, uint32_t, uint32_t, uint32_t, uint32_t) {},
            [](void *, wl_keyboard *, int32_t, int32_t) {},
        };
        wl_keyboard_add_listener(keyboard, &listener, this);
        keyboards.push_back(keyboard);
    }
};

void keymap(SeatInterface &seat)
{
    auto *context = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
    auto *map = xkb_keymap_new_from_names(context, nullptr, XKB_KEYMAP_COMPILE_NO_FLAGS);
    char *text = xkb_keymap_get_as_string(map, XKB_KEYMAP_FORMAT_TEXT_V1);
    seat.keyboard()->setKeymap(QByteArray(text));
    free(text);
    xkb_keymap_unref(map);
    xkb_context_unref(context);
}

void press(KWin::Display &display, SeatInterface &seat, uint32_t key)
{
    seat.notifyKeyboardKey(key, KeyboardKeyState::Pressed, display.nextSerial());
    seat.notifyKeyboardKey(key, KeyboardKeyState::Released, display.nextSerial());
}
}

class SeatBindingsTest : public QObject
{
    Q_OBJECT
private Q_SLOTS:
    void firstKeyWaitsForActualBinding()
    {
        KWin::Display display;
        CompositorInterface compositor(&display);
        SurfaceInterface *surface = nullptr;
        connect(&compositor, &CompositorInterface::surfaceCreated, this, [&](auto *created) { surface = created; });
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

    void bindingsAreClientAndSeatSpecific()
    {
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
