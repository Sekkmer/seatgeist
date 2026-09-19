// SPDX-License-Identifier: MIT OR Apache-2.0
#include <QApplication>
#include <QLineEdit>
#include <QTextStream>
#include <QTimer>
#include <QSocketNotifier>
#include <wayland-client.h>
#include <cstring>

int main(int argc, char **argv)
{
    QApplication app(argc, argv);
    // Independent observer connection records global lifetimes across daemon
    // restarts without depending on Qt's private Wayland implementation.
    auto *display = wl_display_connect(nullptr);
    if (!display) { return 2; }
    auto *registry = wl_display_get_registry(display);
    static const wl_registry_listener listener{
        [](void *, wl_registry *, uint32_t name, const char *interface, uint32_t) {
            if (std::strcmp(interface, "wl_seat") == 0) {
                QTextStream(stdout) << "seat-global=" << name << Qt::endl;
            }
        },
        [](void *, wl_registry *, uint32_t name) {
            QTextStream(stdout) << "global-removed=" << name << Qt::endl;
        },
    };
    wl_registry_add_listener(registry, &listener, nullptr);
    wl_display_roundtrip(display);
    QSocketNotifier notifier(wl_display_get_fd(display), QSocketNotifier::Read);
    QObject::connect(&notifier, &QSocketNotifier::activated, [&] {
        if (wl_display_dispatch(display) < 0) { app.exit(3); }
    });
    QApplication::setDesktopFileName(QStringLiteral("org.seatgeist.FocusFixture"));
    QLineEdit target;
    QLineEdit second;
    QLineEdit other;
    target.setWindowTitle(QStringLiteral("Seatgeist first-input target"));
    other.setWindowTitle(QStringLiteral("Seatgeist physical-focus sentinel"));
    second.setWindowTitle(QStringLiteral("Seatgeist second-input target"));
    target.resize(480, 120);
    second.resize(480, 120);
    other.resize(480, 120);
    QObject::connect(&target, &QLineEdit::textChanged, [](const QString &text) {
        QTextStream(stdout) << "target=" << text << Qt::endl;
    });
    QObject::connect(&other, &QLineEdit::textChanged, [](const QString &text) {
        QTextStream(stdout) << "sentinel=" << text << Qt::endl;
    });
    QObject::connect(&second, &QLineEdit::textChanged, [](const QString &text) {
        QTextStream(stdout) << "second=" << text << Qt::endl;
    });
    target.show();
    second.show();
    other.show();
    QTimer::singleShot(100, &other, [&other] { other.activateWindow(); });
    const int result = app.exec();
    wl_registry_destroy(registry);
    wl_display_disconnect(display);
    return result;
}
