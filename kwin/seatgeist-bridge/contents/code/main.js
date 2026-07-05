const SERVICE = "org.seatgeist.KWinBridge";
const PATH = "/org/seatgeist/KWinBridge1";
const INTERFACE = "org.seatgeist.KWinBridge1";

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

workspace.windowActivated.connect(function (window) {
  publishActiveWindow(window);
  publishWindows();
});

workspace.windowRemoved.connect(function () {
  publishActiveWindow(workspace.activeWindow);
  publishWindows();
});

workspace.windowAdded.connect(function () {
  publishActiveWindow(workspace.activeWindow);
  publishWindows();
});

publishActiveWindow(workspace.activeWindow);
publishWindows();
