use crate::{
    codex_config::{
        ensure_remote_control_enabled as ensure_config_remote_control_enabled,
        remove_remote_control_config,
    },
    json_util::bool_field,
    session_sync_diagnostics::log_session_sync_event,
    settings::{read_settings_value, update_settings_value},
};
use serde_json::{json, Value};

const REMOTE_CONTROL_HOOK_SETTING_KEY: &str = "codex_remote_control_hook_enabled";

pub(crate) const CODEX_REMOTE_CONTROL_HOOK_SCRIPT: &str = r###"
(() => {
  const version = "1";
  const key = "local_app_server_feature_enablement";
  const featureName = "remote_control";

  if (window.__codexSwitchRemoteControlHook?.version === version) {
    window.__codexSwitchRemoteControlHook.activate?.();
    return;
  }

  const state = {
    version,
    activated: false,
    bridgePatched: false,
    errors: [],
    originalBridge: null,
    originalJsonStringify: null,
  };
  window.__codexSwitchRemoteControlHook = state;

  function recordError(error) {
    state.errors.push(String(error?.stack || error));
  }

  function cloneObject(value) {
    if (!value || typeof value !== "object") return {};
    return Array.isArray(value) ? { ...value } : { ...value };
  }

  function forceEnablementValue(value) {
    return {
      ...cloneObject(value),
      [featureName]: true,
    };
  }

  function parseBody(body) {
    if (typeof body !== "string") return { parsed: body, isJsonString: false };
    try {
      return { parsed: JSON.parse(body), isJsonString: true };
    } catch (_error) {
      return { parsed: body, isJsonString: false };
    }
  }

  function bodyWithRemoteControlEnabled(body) {
    const { parsed, isJsonString } = parseBody(body);
    if (!parsed || typeof parsed !== "object" || parsed.featureName !== featureName) {
      return body;
    }
    const next = { ...parsed, enabled: true };
    return isJsonString ? JSON.stringify(next) : next;
  }

  function normalizeStringifyValue(value) {
    if (
      value
      && typeof value === "object"
      && !Array.isArray(value)
      && value.featureName === featureName
    ) {
      return { ...value, enabled: true };
    }
    return value;
  }

  function installJsonStringifyPatch() {
    if (JSON.__codexSwitchRemoteControlHookPatched) return;
    state.originalJsonStringify = JSON.stringify.bind(JSON);
    JSON.stringify = (value, replacer, space) => {
      return state.originalJsonStringify(normalizeStringifyValue(value), replacer, space);
    };
    JSON.__codexSwitchRemoteControlHookPatched = true;
    JSON.__codexSwitchRemoteControlHookVersion = version;
  }

  function normalizeOutboundMessage(message) {
    if (!message || typeof message !== "object") return message;

    if (message.type === "shared-object-set" && message.key === key) {
      return {
        ...message,
        value: forceEnablementValue(message.value),
      };
    }

    if (message.type === "set-local-app-server-feature-enablement") {
      if (message.featureName === featureName || message.params?.featureName === featureName) {
        return {
          ...message,
          enabled: true,
          params: message.params ? { ...message.params, enabled: true } : message.params,
        };
      }
      return message;
    }

    if (
      (message.type === "fetch" || message.type === "fetch-stream")
      && typeof message.url === "string"
      && message.url.includes("set-local-app-server-feature-enablement")
    ) {
      return {
        ...message,
        body: bodyWithRemoteControlEnabled(message.body),
      };
    }

    return message;
  }

  function normalizeInboundSharedObjectEvent(event) {
    const data = event?.data;
    if (
      !data
      || data.__codexSwitchRemoteControlHookForwarded === true
      || data.type !== "shared-object-updated"
      || data.key !== key
    ) {
      return;
    }

    const nextValue = forceEnablementValue(data.value);
    try {
      data.value = nextValue;
      return;
    } catch (_error) {
      event.stopImmediatePropagation();
      window.dispatchEvent(new MessageEvent("message", {
        data: {
          ...data,
          value: nextValue,
          __codexSwitchRemoteControlHookForwarded: true,
        },
      }));
    }
  }

  function installBridgeWrapper() {
    const bridge = window.electronBridge;
    if (!bridge || bridge.__codexSwitchRemoteControlHookPatched) return Boolean(bridge);
    if (typeof bridge.sendMessageFromView !== "function") return false;

    const wrapper = Object.create(bridge);
    for (const prop of Reflect.ownKeys(bridge)) {
      if (prop === "sendMessageFromView" || prop === "getSharedObjectSnapshotValue") continue;
      const descriptor = Object.getOwnPropertyDescriptor(bridge, prop);
      if (descriptor) {
        try {
          Object.defineProperty(wrapper, prop, descriptor);
        } catch (_error) {
          wrapper[prop] = bridge[prop];
        }
      }
    }

    Object.defineProperty(wrapper, "sendMessageFromView", {
      value: async (message) => bridge.sendMessageFromView(normalizeOutboundMessage(message)),
      configurable: true,
      enumerable: true,
      writable: true,
    });
    if (typeof bridge.getSharedObjectSnapshotValue === "function") {
      Object.defineProperty(wrapper, "getSharedObjectSnapshotValue", {
        value: (snapshotKey) => {
          const value = bridge.getSharedObjectSnapshotValue(snapshotKey);
          return snapshotKey === key ? forceEnablementValue(value) : value;
        },
        configurable: true,
        enumerable: true,
        writable: true,
      });
    }
    Object.defineProperty(wrapper, "__codexSwitchRemoteControlHookPatched", {
      value: true,
      configurable: true,
    });
    Object.defineProperty(wrapper, "__codexSwitchRemoteControlHookVersion", {
      value: version,
      configurable: true,
    });

    try {
      Object.defineProperty(window, "electronBridge", {
        value: wrapper,
        configurable: true,
        enumerable: true,
        writable: true,
      });
    } catch (_error) {
      try {
        window.electronBridge = wrapper;
      } catch (_assignError) {
      }
    }

    if (window.electronBridge === wrapper) return true;
    return installDirectBridgePatch(bridge);
  }

  function installDirectBridgePatch(bridge) {
    try {
      const originalSend = bridge.sendMessageFromView.bind(bridge);
      bridge.sendMessageFromView = async (message) => originalSend(normalizeOutboundMessage(message));
      if (typeof bridge.getSharedObjectSnapshotValue === "function") {
        const originalSnapshot = bridge.getSharedObjectSnapshotValue.bind(bridge);
        bridge.getSharedObjectSnapshotValue = (snapshotKey) => {
          const value = originalSnapshot(snapshotKey);
          return snapshotKey === key ? forceEnablementValue(value) : value;
        };
      }
      bridge.__codexSwitchRemoteControlHookPatched = true;
      bridge.__codexSwitchRemoteControlHookVersion = version;
      return bridge.__codexSwitchRemoteControlHookPatched === true;
    } catch (_error) {
      return false;
    }
  }

  function publishEnabledState() {
    const bridge = window.electronBridge;
    if (typeof bridge?.sendMessageFromView !== "function") return;
    bridge.sendMessageFromView({
      type: "shared-object-set",
      key,
      value: { [featureName]: true },
    }).catch(recordError);
  }

  function tryPatchBridge() {
    if (state.bridgePatched) return true;
    state.bridgePatched = installBridgeWrapper();
    if (state.bridgePatched) publishEnabledState();
    return state.bridgePatched;
  }

  state.activate = () => {
    try {
      installJsonStringifyPatch();
      if (!state.activated) {
        window.addEventListener("message", normalizeInboundSharedObjectEvent, true);
        state.activated = true;
      }
      publishEnabledState();
      tryPatchBridge();
      return true;
    } catch (error) {
      recordError(error);
      return false;
    }
  };

  state.activate();
  const timer = setInterval(() => {
    if (tryPatchBridge()) clearInterval(timer);
  }, 100);
  setTimeout(() => clearInterval(timer), 10000);
})();
"###;

pub(crate) fn remote_control_hook_enabled_from_settings(settings: &Value) -> bool {
    bool_field(settings, REMOTE_CONTROL_HOOK_SETTING_KEY)
}

pub(crate) fn prepare_remote_control_hook(context: &str) -> Result<bool, String> {
    let settings = read_settings_value()?;
    if !remote_control_hook_enabled_from_settings(&settings) {
        return Ok(false);
    }

    let changed = ensure_config_remote_control_enabled()?;
    if changed {
        log_session_sync_event(
            "codex_remote_control_hook_prepared",
            json!({
                "context": context,
                "remoteControl": true
            }),
        );
    }
    Ok(changed)
}

#[tauri::command]
pub(crate) fn set_codex_remote_control_hook_enabled(enabled: bool) -> Result<Value, String> {
    let changed = if enabled {
        ensure_config_remote_control_enabled()?
    } else {
        remove_remote_control_config()?
    };
    let settings = update_settings_value(&json!({
        REMOTE_CONTROL_HOOK_SETTING_KEY: enabled
    }))?;
    let settings = super::codex_app::apply_codex_proxy_env_state_to_settings(settings)?;

    Ok(json!({
        "ok": true,
        "message": if enabled {
            "远程控制（手机app）已启用"
        } else {
            "远程控制（手机app）已关闭"
        },
        "settings": settings,
        "changed": changed
    }))
}
