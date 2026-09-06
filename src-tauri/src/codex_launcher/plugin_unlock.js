(async () => {
  const version = "8";
  const existing = window.__codexSwitchPluginUnlockController;
  if (existing?.version === version && !existing.stopped) {
    return await window.__codexSwitchPluginUnlockPatch();
  }
  existing?.stop?.();

  const status = { version, patched: false, attempts: 0, matchedRequests: 0, error: "" };
  const controller = {
    version, stopped: false, timeout: null, dispatcher: null,
    originalDispatch: null, patchedDispatch: null,
    stop() {
      this.stopped = true;
      clearTimeout(this.timeout);
      if (this.dispatcher?.dispatchMessage === this.patchedDispatch) {
        this.dispatcher.dispatchMessage = this.originalDispatch;
      }
      status.patched = false;
    },
  };
  window.__codexSwitchPluginUnlockVersion = version;
  window.__codexSwitchPluginUnlockStatus = status;
  window.__codexSwitchPluginUnlockController = controller;

  function assetUrl() {
    const urls = [
      ...Array.from(document.querySelectorAll("script[src]"), node => node.src),
      ...Array.from(document.querySelectorAll("link[href]"), node => node.href),
      ...performance.getEntriesByType("resource").map(entry => entry.name),
    ];
    return urls.find(url => /\/assets\/app-initial-[\w-]+\.js(?:\?|$)/.test(url)) || "";
  }

  function patchDispatcher(dispatcher) {
    const original = dispatcher.dispatchMessage;
    const patched = function codexSwitchPluginCatalog(type, payload, ...rest) {
      const request = payload?.request;
      const kinds = request?.params?.marketplaceKinds;
      if (type === "mcp-request" && request?.method === "plugin/list"
          && Array.isArray(kinds) && kinds.length === 2
          && kinds.includes("local") && kinds.includes("vertical")) {
        const params = { ...request.params };
        delete params.marketplaceKinds;
        payload = { ...payload, request: { ...request, params } };
        status.matchedRequests += 1;
      }
      return original.call(this, type, payload, ...rest);
    };
    dispatcher.dispatchMessage = patched;
    if (dispatcher.dispatchMessage !== patched) throw new Error("Plugin dispatcher is not writable");
    controller.dispatcher = dispatcher;
    controller.originalDispatch = original;
    controller.patchedDispatch = patched;
  }

  let pending = null;
  async function patch() {
    if (controller.stopped || status.patched) return { ...status };
    if (pending) return await pending;
    clearTimeout(controller.timeout);
    pending = (async () => {
      status.attempts += 1;
      try {
        const url = assetUrl();
        if (!url) throw new Error("Codex app-initial asset not ready");
        const module = await import(url);
        if (controller.stopped) return { ...status };
        const candidates = [...new Set(Object.values(module))].filter(value => value
          && typeof value === "object"
          && typeof value.dispatchMessage === "function"
          && typeof value.subscribe === "function"
          && typeof value.deliverMessage === "function");
        if (candidates.length !== 1) throw new Error(`Expected one Plugin dispatcher, found ${candidates.length}`);
        patchDispatcher(candidates[0]);
        status.patched = true;
        status.error = "";
      } catch (error) {
        status.error = error?.message || String(error);
      }
      if (!controller.stopped && !status.patched && status.attempts < 40) {
        controller.timeout = setTimeout(() => { void patch(); }, 250);
      }
      return { ...status };
    })();
    try { return await pending; } finally { pending = null; }
  }
  window.__codexSwitchPluginUnlockPatch = patch;
  return await patch();
})();
