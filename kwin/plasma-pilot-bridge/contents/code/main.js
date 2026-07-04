const SERVICE = "org.plasmapilot.KWinBridge";
const PATH = "/org/plasmapilot/KWinBridge1";
const INTERFACE = "org.plasmapilot.KWinBridge1";

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

function payloadFor(window) {
  if (!window) {
    return { active: false };
  }
  return {
    active: true,
    id: windowId(window),
    title: maybeString(window.caption) || "",
    app_id: maybeString(window.desktopFileName) || maybeString(window.resourceClass),
    pid: maybeInteger(window.pid),
    geometry: geometryFor(window),
  };
}

function publishActiveWindow(window) {
  const payload = JSON.stringify(payloadFor(window || workspace.activeWindow));
  callDBus(SERVICE, PATH, INTERFACE, "UpdateActiveWindow", payload);
}

workspace.windowActivated.connect(function (window) {
  publishActiveWindow(window);
});

workspace.windowRemoved.connect(function () {
  publishActiveWindow(workspace.activeWindow);
});

workspace.windowAdded.connect(function () {
  publishActiveWindow(workspace.activeWindow);
});

publishActiveWindow(workspace.activeWindow);
