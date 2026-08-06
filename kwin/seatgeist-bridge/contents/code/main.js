const SERVICE = "org.seatgeist.KWinBridge";
const PATH = "/org/seatgeist/KWinBridge1";
const INTERFACE = "org.seatgeist.KWinBridge1";
const DEFAULT_SNAPSHOT_INTERVAL_MS = 2000;
const MIN_SNAPSHOT_INTERVAL_MS = 250;
const MAX_SNAPSHOT_INTERVAL_MS = 60000;
const ACTION_POLL_INTERVAL_MS = 1000;
const ACTION_POLL_STALE_MS = 5000;
const ACTION_SETTLE_MS = 250;
const LAUNCH_APPLICATION_SETTLE_MS = 750;
const MAX_LAUNCH_TIMEOUT_MS = 30000;
var launchIntents = [];

function maybeString(value) {
  if (value === undefined || value === null) {
    return null;
  }
  const text = String(value);
  return text.length === 0 ? null : text;
}

function maybeInteger(value) {
  const number = Number(value);
  if (!isFinite(number)) {
    return null;
  }
  return Math.round(number);
}

function geometryFor(window) {
  if (!window) {
    return null;
  }
  const width = Math.max(1, maybeInteger(window.width) || 1);
  const height = Math.max(1, maybeInteger(window.height) || 1);
  return {
    x: maybeInteger(window.x) || 0,
    y: maybeInteger(window.y) || 0,
    width: width,
    height: height,
  };
}

function windowId(window) {
  return (
    maybeString(window.internalId) ||
    maybeString(window.uuid) ||
    maybeString(window.windowId)
  );
}

function windowPayload(window) {
  if (!window) {
    return null;
  }
  return {
    id: windowId(window),
    title: maybeString(window.caption) || "",
    app_id: maybeString(window.desktopFileName) || maybeString(window.resourceClass),
    pid: maybeInteger(window.pid),
    geometry: geometryFor(window),
  };
}

function payloadFor(window) {
  const payload = windowPayload(window);
  if (!payload) {
    return { active: false };
  }
  payload.active = true;
  return payload;
}

function publishActiveWindow(window) {
  const payload = JSON.stringify(payloadFor(window || workspace.activeWindow));
  callDBus(SERVICE, PATH, INTERFACE, "UpdateActiveWindow", payload);
}

function publishWindows() {
  const source = workspace.stackingOrder || [];
  const windows = [];
  for (var i = 0; i < source.length; i++) {
    const payload = windowPayload(source[i]);
    if (payload && payload.id) {
      windows.push(payload);
    }
  }
  callDBus(SERVICE, PATH, INTERFACE, "UpdateWindows", JSON.stringify({ windows: windows }));
}

function publishSnapshot() {
  callDBus(SERVICE, PATH, INTERFACE, "RegisterActionCapabilities", "resize_window,move_window,launch_window,close_window");
  publishActiveWindow(workspace.activeWindow);
  publishWindows();
}

function findWindowById(id) {
  const windows = workspace.stackingOrder || [];
  for (var i = 0; i < windows.length; i++) {
    if (windowId(windows[i]) === id) {
      return windows[i];
    }
  }
  return null;
}

function completeAction(result) {
  callDBus(SERVICE, PATH, INTERFACE, "CompleteAction", JSON.stringify(result));
}

function normalizeDesktopEntry(value) {
  const text = maybeString(value);
  if (!text) {
    return null;
  }
  return text.toLowerCase().replace(/\.desktop$/, "");
}

function outputByName(name) {
  const target = maybeString(name);
  const screens = workspace.screens || [];
  if (!target) {
    return null;
  }
  for (var i = 0; i < screens.length; i++) {
    if (maybeString(screens[i].name) === target) {
      return screens[i];
    }
  }
  return null;
}

function anchoredLaunchGeometry(intent, area, width, height) {
  const margin = Math.max(0, maybeInteger(intent.margin) || 0);
  var x = maybeInteger(area.x) + margin;
  var y = maybeInteger(area.y) + margin;
  if (intent.anchor === "top_right" || intent.anchor === "bottom_right") {
    x = maybeInteger(area.x) + maybeInteger(area.width) - width - margin;
  } else if (intent.anchor === "center") {
    x = maybeInteger(area.x) + Math.round((maybeInteger(area.width) - width) / 2);
  }
  if (intent.anchor === "bottom_left" || intent.anchor === "bottom_right") {
    y = maybeInteger(area.y) + maybeInteger(area.height) - height - margin;
  } else if (intent.anchor === "center") {
    y = maybeInteger(area.y) + Math.round((maybeInteger(area.height) - height) / 2);
  }
  return { x: x, y: y, width: width, height: height };
}

function desiredLaunchGeometry(intent, window) {
  const area = workspace.clientArea(KWin.PlacementArea, window);
  const current = geometryFor(window);
  const margin = Math.max(0, maybeInteger(intent.margin) || 0);
  const availableWidth = Math.max(1, maybeInteger(area.width) - margin * 2);
  const availableHeight = Math.max(1, maybeInteger(area.height) - margin * 2);
  const requestedWidth = maybeInteger(intent.width);
  const requestedHeight = maybeInteger(intent.height);
  const width = Math.min(availableWidth, Math.max(1, requestedWidth || current.width));
  const height = Math.min(availableHeight, Math.max(1, requestedHeight || current.height));
  return anchoredLaunchGeometry(intent, area, width, height);
}

function applyLaunchIntent(intent, window) {
  const output = outputByName(intent.monitor_id);
  if (output && typeof workspace.sendClientToScreen === "function") {
    workspace.sendClientToScreen(window, output);
  }
  const desired = desiredLaunchGeometry(intent, window);
  window.frameGeometry = desired;
  const constrained = geometryFor(window);
  if (constrained.width !== desired.width || constrained.height !== desired.height) {
    window.frameGeometry = anchoredLaunchGeometry(
      intent,
      workspace.clientArea(KWin.PlacementArea, window),
      constrained.width,
      constrained.height,
    );
  }
  if (intent.activation === "preserve_focus") {
    const previous = findWindowById(intent.previous_window_id);
    if (previous) {
      workspace.activeWindow = previous;
    }
  } else if (intent.activation === "activate") {
    workspace.activeWindow = window;
  }
  return geometryFor(window);
}

function finishLaunchIntent(intent, window) {
  if (typeof QTimer === "undefined") {
    completeAction({ id: intent.id, ok: false, error: "QTimer is unavailable for launch verification" });
    return;
  }
  var firstTimer = new QTimer();
  firstTimer.timeout.connect(function () {
    firstTimer.stop();
    const expected = applyLaunchIntent(intent, window);
    var verifyTimer = new QTimer();
    verifyTimer.timeout.connect(function () {
      verifyTimer.stop();
      const actual = geometryFor(window);
      const previous = findWindowById(intent.previous_window_id);
      const focusPreserved =
        (!intent.previous_window_id && !workspace.activeWindow) ||
        (previous && windowId(workspace.activeWindow) === intent.previous_window_id);
      const activationConfirmed =
        intent.activation === "preserve_focus"
          ? focusPreserved
          : windowId(workspace.activeWindow) === windowId(window);
      const placementConfirmed =
        Math.abs(actual.x - expected.x) <= 1 &&
        Math.abs(actual.y - expected.y) <= 1;
      completeAction({
        id: intent.id,
        ok: placementConfirmed && activationConfirmed,
        error: placementConfirmed
          ? (activationConfirmed ? null : "requested focus policy was not retained")
          : "KWin did not retain the anchored window position",
        geometry: actual,
        window_id: windowId(window),
        app_id: maybeString(window.desktopFileName) || maybeString(window.resourceClass),
        title: maybeString(window.caption) || "",
        pid: maybeInteger(window.pid),
        monitor_id: window.output ? maybeString(window.output.name) : null,
        focus_preserved: focusPreserved,
      });
      publishSnapshot();
    });
    verifyTimer.start(ACTION_SETTLE_MS);
  });
  firstTimer.start(LAUNCH_APPLICATION_SETTLE_MS);
}

function handleLaunchWindowAdded(window) {
  const app = normalizeDesktopEntry(window.desktopFileName) || normalizeDesktopEntry(window.resourceClass);
  if (!app || window.specialWindow) {
    return;
  }
  for (var i = 0; i < launchIntents.length; i++) {
    if (launchIntents[i].desktop_entry === app) {
      const intent = launchIntents.splice(i, 1)[0];
      applyLaunchIntent(intent, window);
      finishLaunchIntent(intent, window);
      return;
    }
  }
}

function pruneLaunchIntents() {
  const now = Date.now();
  const retained = [];
  for (var i = 0; i < launchIntents.length; i++) {
    if (launchIntents[i].expires_at <= now) {
      completeAction({ id: launchIntents[i].id, ok: false, error: "launch intent expired before a matching window appeared" });
    } else {
      retained.push(launchIntents[i]);
    }
  }
  launchIntents = retained;
}

function handleLaunchWindow(action) {
  const desktopEntry = normalizeDesktopEntry(action.desktop_entry);
  const timeoutMs = Math.min(MAX_LAUNCH_TIMEOUT_MS, Math.max(1000, maybeInteger(action.timeout_ms) || 10000));
  if (!desktopEntry) {
    completeAction({ id: action.id, ok: false, error: "invalid desktop entry" });
    return;
  }
  launchIntents.push({
    id: action.id,
    desktop_entry: desktopEntry,
    anchor: maybeString(action.anchor) || "top_left",
    monitor_id: maybeString(action.monitor_id),
    width: maybeInteger(action.width),
    height: maybeInteger(action.height),
    margin: Math.max(0, maybeInteger(action.margin) || 0),
    activation: maybeString(action.activation) || "preserve_focus",
    previous_window_id: windowId(workspace.activeWindow),
    expires_at: Date.now() + timeoutMs,
  });
  callDBus(SERVICE, PATH, INTERFACE, "AcknowledgeAction", action.id);
}

function handleCancelLaunchWindow(action) {
  const launchId = maybeString(action.launch_id);
  if (!launchId) {
    return;
  }
  const retained = [];
  for (var i = 0; i < launchIntents.length; i++) {
    if (launchIntents[i].id !== launchId) {
      retained.push(launchIntents[i]);
    }
  }
  launchIntents = retained;
}

function completeResizeAfterSettle(action, window, previous, width, height) {
  if (typeof QTimer === "undefined") {
    completeAction({
      id: action.id,
      ok: false,
      error: "QTimer is unavailable for resize verification",
    });
    return;
  }
  var settleTimer = new QTimer();
  settleTimer.timeout.connect(function () {
    settleTimer.stop();
    const actual = geometryFor(window);
    const requestedChange = previous.width !== width || previous.height !== height;
    const observedChange =
      actual.width !== previous.width || actual.height !== previous.height;
    if (requestedChange && !observedChange) {
      completeAction({
        id: action.id,
        ok: false,
        error: "KWin did not apply the requested window size",
        geometry: actual,
      });
    } else {
      completeAction({ id: action.id, ok: true, geometry: actual });
    }
    publishSnapshot();
  });
  settleTimer.start(ACTION_SETTLE_MS);
}

function completeMoveAfterSettle(action, window, previous, x, y) {
  if (typeof QTimer === "undefined") {
    completeAction({
      id: action.id,
      ok: false,
      error: "QTimer is unavailable for move verification",
    });
    return;
  }
  var settleTimer = new QTimer();
  settleTimer.timeout.connect(function () {
    settleTimer.stop();
    const actual = geometryFor(window);
    const requestedChange = previous.x !== x || previous.y !== y;
    const observedChange = actual.x !== previous.x || actual.y !== previous.y;
    if (requestedChange && !observedChange) {
      completeAction({
        id: action.id,
        ok: false,
        error: "KWin did not apply the requested window position",
        geometry: actual,
      });
    } else {
      completeAction({ id: action.id, ok: true, geometry: actual });
    }
    publishSnapshot();
  });
  settleTimer.start(ACTION_SETTLE_MS);
}

function handleResizeWindow(action) {
  const window = findWindowById(maybeString(action.window_id));
  if (!window) {
    completeAction({ id: action.id, ok: false, error: "window not found" });
    return;
  }
  if (window.specialWindow || window.resizeable === false) {
    completeAction({ id: action.id, ok: false, error: "window is not resizeable" });
    return;
  }
  const width = maybeInteger(action.width);
  const height = maybeInteger(action.height);
  if (!width || !height || width < 64 || height < 64 || width > 32768 || height > 32768) {
    completeAction({ id: action.id, ok: false, error: "invalid logical window size" });
    return;
  }
  const current = geometryFor(window);
  window.frameGeometry = {
    x: current.x,
    y: current.y,
    width: width,
    height: height,
  };
  completeResizeAfterSettle(action, window, current, width, height);
}

function handleMoveWindow(action) {
  const window = findWindowById(maybeString(action.window_id));
  if (!window) {
    completeAction({ id: action.id, ok: false, error: "window not found" });
    return;
  }
  if (window.specialWindow || window.moveable === false) {
    completeAction({ id: action.id, ok: false, error: "window is not moveable" });
    return;
  }
  const x = maybeInteger(action.x);
  const y = maybeInteger(action.y);
  if (x === null || y === null) {
    completeAction({ id: action.id, ok: false, error: "invalid logical window position" });
    return;
  }
  const current = geometryFor(window);
  window.frameGeometry = {
    x: x,
    y: y,
    width: current.width,
    height: current.height,
  };
  completeMoveAfterSettle(action, window, current, x, y);
}

function handleCloseWindow(action) {
  const requestedId = maybeString(action.window_id);
  const window = findWindowById(requestedId);
  if (!window) {
    completeAction({ id: action.id, ok: false, error: "window not found" });
    return;
  }
  if (window.specialWindow || typeof window.closeWindow !== "function") {
    completeAction({ id: action.id, ok: false, error: "window does not support exact compositor close" });
    return;
  }
  if (typeof QTimer === "undefined") {
    completeAction({ id: action.id, ok: false, error: "QTimer is unavailable for close verification" });
    return;
  }
  window.closeWindow();
  var settleTimer = new QTimer();
  settleTimer.timeout.connect(function () {
    settleTimer.stop();
    if (findWindowById(requestedId)) {
      completeAction({
        id: action.id,
        ok: false,
        error: "exact target window remained open after compositor close request",
      });
      return;
    }
    completeAction({ id: action.id, ok: true, window_id: requestedId });
    publishSnapshot();
  });
  settleTimer.start(ACTION_SETTLE_MS);
}

function handleActionPayload(payload) {
  if (!payload) {
    return;
  }
  var action;
  try {
    action = JSON.parse(payload);
  } catch (error) {
    return;
  }
  if (!action || !maybeString(action.id)) {
    return;
  }
  if (action.action === "resize_window") {
    handleResizeWindow(action);
    return;
  }
  if (action.action === "move_window") {
    handleMoveWindow(action);
    return;
  }
  if (action.action === "close_window") {
    handleCloseWindow(action);
    return;
  }
  if (action.action === "launch_window") {
    handleLaunchWindow(action);
    return;
  }
  if (action.action === "cancel_launch_window") {
    handleCancelLaunchWindow(action);
    return;
  }
  completeAction({ id: action.id, ok: false, error: "unsupported bridge action" });
}

function snapshotIntervalMs() {
  const configured = maybeInteger(
    readConfig("SnapshotIntervalMs", DEFAULT_SNAPSHOT_INTERVAL_MS),
  );
  return Math.max(
    MIN_SNAPSHOT_INTERVAL_MS,
    Math.min(MAX_SNAPSHOT_INTERVAL_MS, configured || DEFAULT_SNAPSHOT_INTERVAL_MS),
  );
}

// KWin exposes QTimer to script JavaScript. The heartbeat repairs daemon-only
// restarts because callDBus cannot notify this script when its destination
// service returns. Event-driven publishing remains the compatibility fallback.
var snapshotTimer = null;
function startSnapshotHeartbeat() {
  if (typeof QTimer === "undefined") {
    return;
  }
  snapshotTimer = new QTimer();
  snapshotTimer.timeout.connect(publishSnapshot);
  snapshotTimer.start(snapshotIntervalMs());
}

var actionTimer = null;
var actionPollInFlight = false;
var actionPollStartedAtMs = 0;
var actionPollRearmPending = false;
function pollPendingAction() {
  if (actionPollRearmPending && actionTimer !== null) {
    actionPollRearmPending = false;
    actionTimer.stop();
    actionTimer.start(ACTION_POLL_INTERVAL_MS);
  }
  pruneLaunchIntents();
  const now = Date.now();
  if (actionPollInFlight && now - actionPollStartedAtMs < ACTION_POLL_STALE_MS) {
    return;
  }
  actionPollInFlight = true;
  actionPollStartedAtMs = now;
  callDBus(SERVICE, PATH, INTERFACE, "TakePendingAction", function (payload) {
    actionPollInFlight = false;
    actionPollStartedAtMs = 0;
    if (typeof payload !== "string") {
      return;
    }
    handleActionPayload(payload);
    // TakePendingAction is a daemon-side long poll. A successful return opens
    // the next wait on the next event-loop turn. Reusing the watchdog timer
    // avoids synchronous callback recursion in script hosts and test shims.
    actionPollRearmPending = true;
    if (actionTimer !== null) {
      actionTimer.stop();
      actionTimer.start(1);
    }
  });
}

function startActionPolling() {
  if (typeof QTimer === "undefined") {
    return;
  }
  actionTimer = new QTimer();
  actionTimer.timeout.connect(pollPendingAction);
  actionTimer.start(ACTION_POLL_INTERVAL_MS);
}

workspace.windowActivated.connect(function (window) {
  publishActiveWindow(window);
  publishWindows();
});

workspace.windowRemoved.connect(function () {
  publishSnapshot();
});

workspace.windowAdded.connect(function (window) {
  handleLaunchWindowAdded(window);
  publishSnapshot();
});

publishSnapshot();
startSnapshotHeartbeat();
startActionPolling();
