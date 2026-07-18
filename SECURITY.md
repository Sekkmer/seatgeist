# Security Policy

Seatgeist is security-sensitive desktop-control software. A defect can expose
screen content, direct input to the wrong window, weaken an approval boundary,
or disclose journal metadata even when the daemon remains user-scoped.

## Supported versions

The project is currently pre-release. Only the current default development
branch is under active security maintenance until a public versioning policy is
published.

## Reporting a vulnerability

Report suspected vulnerabilities privately to `sekkmer@gmail.com`. Please do
not include credentials, clipboard contents, private screenshots, typed text,
or unredacted journal files. A useful report includes:

- the Seatgeist commit and affected component;
- KDE Plasma, KWin, Wayland, and distribution versions;
- the expected policy or isolation boundary;
- minimal reproduction steps using disposable data; and
- redacted logs or compact journal metadata when relevant.

Allow time for acknowledgement and remediation before public disclosure.

## Security expectations

Control actions must pass through the daemon policy engine and action journal.
New backends must preserve active-window guards, panic-stop behavior, ownership
checks, and fail-closed handling before they can be considered supported. A
backend that bypasses those boundaries is a security issue, not a compatibility
fallback.

The user daemon should run as the desktop user, not as root. Optional uinput,
KWin, portal, and desktop-entry integrations must remain explicitly scoped and
document their fallback behavior. See [Safety](docs/safety.md) and the
[threat model](docs/threat-model.md) for the maintained security model.
