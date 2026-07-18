#!/usr/bin/env node

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const root = path.resolve(__dirname, "..");
const bridgePath = path.join(
  root,
  "kwin",
  "seatgeist-bridge",
  "contents",
  "code",
  "main.js",
);
const bridgeSource = fs.readFileSync(bridgePath, "utf8");

function signal() {
  const handlers = [];
  return {
    connect(handler) {
      handlers.push(handler);
    },
    emit(...args) {
      for (const handler of handlers) {
        handler(...args);
      }
    },
  };
}

function runBridge({
  interval = undefined,
  timerAvailable = true,
  pendingActions = [],
  dropFirstActionPoll = false,
  applyResize = true,
} = {}) {
  const calls = [];
  const timers = [];
  const registeredCapabilities = [];
  let nowMs = 0;
  let actionPollCount = 0;
  const activeWindow = {
    internalId: "window-1",
    caption: "Editor",
    desktopFileName: "org.kde.kate",
    pid: 42,
    x: 10,
    y: 20,
    width: 800,
    height: 600,
    resizeable: true,
  };
  Object.defineProperty(activeWindow, "frameGeometry", {
    set(geometry) {
      if (!applyResize) {
        return;
      }
      this.x = geometry.x;
      this.y = geometry.y;
      this.width = geometry.width;
      this.height = geometry.height;
    },
  });
  const workspace = {
    activeWindow,
    stackingOrder: [activeWindow],
    windowActivated: signal(),
    windowRemoved: signal(),
    windowAdded: signal(),
    screens: [{ name: "DP-1" }],
    clientArea() {
      return { x: 0, y: 0, width: 1920, height: 1040 };
    },
    sendClientToScreen(window, output) {
      window.output = output;
    },
  };
  const context = {
    workspace,
    KWin: { PlacementArea: 0 },
    Date: {
      now() {
        return nowMs;
      },
    },
    callDBus(service, objectPath, interfaceName, method, payload) {
      if (method === "RegisterActionCapabilities") {
        registeredCapabilities.push(payload);
        return;
      }
      calls.push({ service, objectPath, interfaceName, method, payload });
      if (method === "TakePendingAction" && typeof payload === "function") {
        actionPollCount += 1;
        if (dropFirstActionPoll && actionPollCount === 1) {
          return;
        }
        payload(pendingActions.shift() || "");
      }
    },
    readConfig(key, fallback) {
      assert.equal(key, "SnapshotIntervalMs");
      return interval === undefined ? fallback : interval;
    },
  };

  if (timerAvailable) {
    context.QTimer = class MockTimer {
      constructor() {
        this.interval = null;
        this.timeout = signal();
        this.started = false;
        timers.push(this);
      }
      start(interval) {
        this.interval = interval;
        this.started = true;
      }
      stop() {
        this.started = false;
      }
    };
  }

  vm.runInNewContext(bridgeSource, context, { filename: bridgePath });
  return {
    calls,
    timers,
    workspace,
    registeredCapabilities,
    advanceTime(milliseconds) {
      nowMs += milliseconds;
    },
  };
}

function assertSnapshot(calls, offset, activeId = "window-1") {
  assert.equal(calls[offset].method, "UpdateActiveWindow");
  assert.equal(JSON.parse(calls[offset].payload).id, activeId);
  assert.equal(calls[offset + 1].method, "UpdateWindows");
  assert.deepEqual(
    JSON.parse(calls[offset + 1].payload).windows.map((window) => window.id),
    ["window-1"],
  );
}

{
  const runtime = runBridge();
  assertSnapshot(runtime.calls, 0);
  assert.deepEqual(runtime.registeredCapabilities, ["resize_window,move_window,launch_window"]);
  assert.equal(runtime.timers.length, 2);
  assert.equal(runtime.timers[0].interval, 2000);
  assert.equal(runtime.timers[0].started, true);
  assert.equal(runtime.timers[1].interval, 50);
  assert.equal(runtime.timers[1].started, true);

  runtime.timers[0].timeout.emit();
  assertSnapshot(runtime.calls, 2);

  runtime.workspace.windowRemoved.emit();
  assertSnapshot(runtime.calls, 4);
}

{
  const runtime = runBridge({
    pendingActions: [
      JSON.stringify({
        id: "move-1",
        action: "move_window",
        window_id: "window-1",
        x: 40,
        y: 60,
      }),
    ],
  });
  runtime.timers[1].timeout.emit();
  assert.equal(runtime.workspace.activeWindow.x, 40);
  assert.equal(runtime.workspace.activeWindow.y, 60);
  runtime.timers[2].timeout.emit();
  const completion = runtime.calls.find((call) => call.method === "CompleteAction");
  assert.deepEqual(JSON.parse(completion.payload), {
    id: "move-1",
    ok: true,
    geometry: { x: 40, y: 60, width: 800, height: 600 },
  });
}

{
  const runtime = runBridge({
    pendingActions: [
      JSON.stringify({
        id: "launch-1",
        action: "launch_window",
        desktop_entry: "org.kde.kcalc",
        anchor: "top_right",
        monitor_id: "DP-1",
        width: 400,
        height: 300,
        margin: 20,
        activation: "preserve_focus",
        timeout_ms: 10000,
      }),
    ],
  });
  const previous = runtime.workspace.activeWindow;
  runtime.timers[1].timeout.emit();
  const acknowledgement = runtime.calls.find((call) => call.method === "AcknowledgeAction");
  assert.equal(acknowledgement.payload, "launch-1");
  assert.equal(
    runtime.calls.some((call) => call.method === "CompleteAction"),
    false,
  );

  const launched = {
    internalId: "window-2",
    caption: "KCalc",
    desktopFileName: "org.kde.kcalc",
    pid: 84,
    x: 100,
    y: 100,
    width: 600,
    height: 500,
    moveable: true,
  };
  Object.defineProperty(launched, "frameGeometry", {
    set(geometry) {
      this.x = geometry.x;
      this.y = geometry.y;
      this.width = geometry.width;
      this.height = geometry.height;
    },
  });
  runtime.workspace.stackingOrder.push(launched);
  runtime.workspace.activeWindow = launched;
  runtime.workspace.windowAdded.emit(launched);
  assert.equal(launched.x, 1500);
  assert.equal(launched.y, 20);
  assert.equal(runtime.workspace.activeWindow, previous);
  assert.equal(runtime.timers[2].interval, 750);
  runtime.timers[2].timeout.emit();
  runtime.timers[3].timeout.emit();
  const completion = runtime.calls.find((call) => call.method === "CompleteAction");
  const result = JSON.parse(completion.payload);
  assert.equal(result.ok, true);
  assert.equal(result.window_id, "window-2");
  assert.equal(result.focus_preserved, true);
  assert.equal(result.monitor_id, "DP-1");
  assert.deepEqual(result.geometry, { x: 1500, y: 20, width: 400, height: 300 });
}

{
  const runtime = runBridge({ interval: 1 });
  assert.equal(runtime.timers[0].interval, 250);
}

{
  const runtime = runBridge({ interval: 999999 });
  assert.equal(runtime.timers[0].interval, 60000);
}

{
  const runtime = runBridge({ timerAvailable: false });
  assertSnapshot(runtime.calls, 0);
  assert.equal(runtime.timers.length, 0);
}

{
  const runtime = runBridge({
    pendingActions: [
      JSON.stringify({
        id: "action-1",
        action: "resize_window",
        window_id: "window-1",
        width: 1280,
        height: 720,
      }),
    ],
  });
  runtime.timers[1].timeout.emit();
  assert.equal(runtime.workspace.activeWindow.width, 1280);
  assert.equal(runtime.workspace.activeWindow.height, 720);
  assert.equal(runtime.timers[2].interval, 250);
  runtime.timers[2].timeout.emit();
  assert.equal(runtime.timers[2].started, false);
  const completion = runtime.calls.find((call) => call.method === "CompleteAction");
  assert.deepEqual(JSON.parse(completion.payload), {
    id: "action-1",
    ok: true,
    geometry: { x: 10, y: 20, width: 1280, height: 720 },
  });
}

{
  const runtime = runBridge({
    dropFirstActionPoll: true,
    pendingActions: [
      JSON.stringify({
        id: "action-after-daemon-restart",
        action: "resize_window",
        window_id: "window-1",
        width: 1024,
        height: 768,
      }),
    ],
  });
  runtime.timers[1].timeout.emit();
  runtime.timers[1].timeout.emit();
  assert.equal(
    runtime.calls.filter((call) => call.method === "TakePendingAction").length,
    1,
  );
  runtime.advanceTime(1000);
  runtime.timers[1].timeout.emit();
  assert.equal(runtime.workspace.activeWindow.width, 1024);
  assert.equal(runtime.workspace.activeWindow.height, 768);
  assert.equal(
    runtime.calls.filter((call) => call.method === "TakePendingAction").length,
    2,
  );
  runtime.timers[2].timeout.emit();
}

{
  const runtime = runBridge({
    applyResize: false,
    pendingActions: [
      JSON.stringify({
        id: "action-not-applied",
        action: "resize_window",
        window_id: "window-1",
        width: 900,
        height: 700,
      }),
    ],
  });
  runtime.timers[1].timeout.emit();
  runtime.timers[2].timeout.emit();
  const completion = runtime.calls.find((call) => call.method === "CompleteAction");
  assert.equal(JSON.parse(completion.payload).ok, false);
  assert.match(JSON.parse(completion.payload).error, /did not apply/);
}

console.log("test-kwin-bridge: ok");
