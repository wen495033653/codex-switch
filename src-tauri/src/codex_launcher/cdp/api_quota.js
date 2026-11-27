(function locateRemoteControlQuota(source, url, configRef) {
  const id = "[A-Za-z_$][\\w$]*";
  const configCondition = (read, data, action, host = '"local"') => `(() => {
    const state = globalThis.__codexSwitchApiQuotaState ??= { enabled: true, unlockCount: 0, hiddenBannerCount: 0, lastProvider: null, lastError: null };
    try {
      if (!state.enabled || ${host} !== "local") return false;
      const config = ${read}?.data;
      const provider = config?.model_provider ?? "openai";
      const mixedProvider = config?.model_providers?.api;
      const mixed = provider === "api" && mixedProvider?.requires_openai_auth === true && typeof mixedProvider.experimental_bearer_token === "string" && mixedProvider.experimental_bearer_token.trim().length > 0;
      state.lastProvider = provider;
      if (mixed && ${data}?.rate_limit?.allowed === false) {
        ${action}
      }
    } catch (error) {
      state.lastError = String(error?.stack ?? error);
    }
    return false;
  })()`;
  const location = (kind, offset, endOffset, condition, details) => {
    const before = source.slice(0, offset).split("\n");
    const end = source.slice(0, endOffset).split("\n");
    return { kind, url, lineNumber: before.length - 1, columnNumber: before.at(-1).length,
      endLineNumber: end.length - 1, endColumnNumber: end.at(-1).length, condition, ...details };
  };

  if (source.includes("as __webpack_require__") && source.includes("assets/async/")) {
    const boot = new RegExp(`var (${id})=(${id})\\.O\\(void 0,\\[`, "g");
    const matches = [...source.matchAll(boot)];
    if (matches.length !== 1) throw new Error(`Rspack runtime bootstrap count=${matches.length}; expected=1`);
    const match = matches[0];
    const req = match[2];
    return location("rspack_runtime", match.index, match.index + match[0].length,
      `((globalThis.__codexSwitchWebpackRequire = ${req}), false)`, { req });
  }

  const isLegacyBanner = url.includes("/assets/composer-content-");
  const isBanner = isLegacyBanner || (source.includes("isUsageBannerEnabled:") && source.includes("rateLimitWarningThreshold:"));
  if (isBanner) {
    // The local composer derives this banner model from the official quota
    // snapshot. Remove only that render-local snapshot, never the shared query.
    const result = new RegExp(`imageGenerationLimit:${id},isUsageBannerEnabled:${id},onDismissImageGenerationLimit:${id},rateLimit:(${id}),rateLimitWarningThreshold:${id},sendBlocked:`, "g");
    if (isLegacyBanner) {
      const results = [...source.matchAll(result)].map(item => {
        const data = item[1];
        const bodyStart = source.lastIndexOf("function ", item.index);
        const body = source.slice(bodyStart, item.index);
        const scope = new RegExp(`\\{hostId:${id},selectedModel:${id}\\}=${id},(${id})=${id}===void 0\\?${id}:${id},(${id})=${id}\\(${id}\\)`).exec(body);
        const read = new RegExp(`\\{data:${data},dataUpdatedAt:${id}\\}=${id}\\(${id}\\)`).exec(body);
        return scope && read ? { data, bodyStart, scope, read } : null;
      }).filter(Boolean);
      if (results.length !== 1) throw new Error(`Remote quota banner model count=${results.length}; expected=1`);
      const { data, bodyStart, scope, read } = results[0];
      const configReads = new RegExp(`let\\{data:(${id})\\}=${id}\\((${id}),${id}\\),${id}=${id}\\?\\?\\1\\?\\.model_provider\\?\\?null`, "g");
      const descriptors = [...new Set([...source.matchAll(configReads)].map(item => item[2]))];
      if (!scope || !read || descriptors.length !== 1) throw new Error(`Remote quota banner scope/config missing; descriptorCount=${descriptors.length}`);
      const [host, store] = scope.slice(1);
      const config = descriptors[0];
      const offset = bodyStart + read.index + read[0].length + 1;
      const endOffset = source.indexOf(";", offset);
      if (endOffset <= offset) throw new Error("Remote quota banner read boundary missing");
      return location("official_quota_banner", offset, endOffset,
        configCondition(`${store}.get(${config}, { hostId: "local", cwd: null })`, data,
          `${data} = null; state.hiddenBannerCount += 1;`, host), { data, config, store, host });
    }

    const results = [...source.matchAll(result)].map(item => {
      const data = item[1];
      const bodyStart = source.lastIndexOf("function(", item.index);
      if (bodyStart < 0) return null;
      const body = source.slice(bodyStart, item.index);
      const scope = new RegExp(`\\{hostId:(${id}),selectedModel:${id}\\}=${id},(${id})=(?:void 0===\\1|\\1===void 0)\\?"local":\\1,(${id})=\\(0,${id}\\.${id}\\)\\(${id}\\.${id}\\)`).exec(body);
      const read = new RegExp(`${data}=\\(0,${id}\\.${id}\\)\\(${id}\\.${id}\\),`).exec(body);
      return scope && read ? { data, bodyStart, host: scope[2], store: scope[3], read } : null;
    }).filter(Boolean);
    if (results.length !== 1) throw new Error(`Remote quota Rspack banner model count=${results.length}; expected=1`);
    const { data, bodyStart, host, store, read } = results[0];
    if (!configRef?.moduleId || !configRef?.exportKey) {
      throw new Error("Remote quota Rspack banner missing saved-config module ref");
    }
    const offset = bodyStart + read.index + read[0].length;
    const endOffset = source.indexOf(";", offset);
    if (endOffset <= offset) throw new Error("Remote quota Rspack banner read boundary missing");
    const atomExpr = `globalThis.__codexSwitchWebpackRequire?.(${JSON.stringify(configRef.moduleId)})?.[${JSON.stringify(configRef.exportKey)}]`;
    return location("official_quota_banner", offset, endOffset,
      configCondition(`((cfgAtom => cfgAtom ? ${store}.get(cfgAtom, { hostId: "local", cwd: null }) : null)(${atomExpr}))`, data,
        `${data} = null; state.hiddenBannerCount += 1;`, host), { data, store, host, configRef });
  }

  // Legacy Vite primary bundle
  if (url.includes("/assets/app-primary-")) {
    const atom = new RegExp(
      `(${id})=${id}\\(${id},\\(\\{get:(${id})\\}\\)=>\\{let (${id})=\\2\\(${id}\\),(${id})=\\2\\(${id}\\)\\.data;if\\(\\3\\.authMethod!==\`chatgpt\``,
      "g",
    );
    const atoms = [...source.matchAll(atom)];
    if (atoms.length !== 1) throw new Error(`API quota selector count=${atoms.length}; expected=1`);
    const match = atoms[0];
    const getter = match[2];
    const data = match[4];
    const body = source.slice(match.index, match.index + 2400);
    const guard = body.indexOf(`${data}.rate_limit?.allowed!==!1`);
    if (guard < 0) throw new Error("API quota selector rate_limit.allowed guard missing");

    // This is the same saved-config descriptor used by the desktop's own local
    // model-provider check, not SW settings or the official account's quota data.
    const configRead = new RegExp(
      `let\\{data:(${id})\\}=(${id})\\.get\\((${id}),\\{cwd:${id},hostId:${id}\\}\\);return\\s*\\1==null\\|\\|\\(\\2\\.get\\(${id},${id}\\)\\?\\?\\1\\.model_provider\\?\\?\`openai\`\\)!==\`openai\``,
      "g",
    );
    const descriptors = [...new Set([...source.matchAll(configRead)].map(item => item[3]))];
    if (descriptors.length !== 1) {
      throw new Error(`API quota config descriptor count=${descriptors.length}; expected=1`);
    }
    const config = descriptors[0];
    const offset = match.index + match[0].lastIndexOf("if(");
    return location("send_gate", offset, match.index + guard,
      configCondition(`${getter}(${config}, { hostId: "local", cwd: null })`, data,
        `${data} = { ...${data}, rate_limit: { ...${data}.rate_limit, allowed: true } }; state.unlockCount += 1;`),
      { atom: match[1], getter, data, config,
        bannerUrls: [...new Set(source.match(/\.\/composer-content-[\w-]+\.js/g) ?? [])]
          .map(relative => url.slice(0, url.lastIndexOf("/") + 1) + relative.slice(2)) });
  }

  // Rspack async primary bundle (Codex 26.1120+)
  const rspackAtom = new RegExp(
    `(${id})=\\(0,${id}\\.${id}\\)\\(${id}\\.${id},\\(\\{get:(${id})\\}\\)=>\\{let (${id})=\\2\\(${id}\\.${id}\\),(${id})=\\2\\(${id}\\.${id}\\)\\.data;if\\((?:"chatgpt"!==\\3\\.authMethod|\\3\\.authMethod!=="chatgpt"|\\3\\.authMethod!==\`chatgpt\`)`,
    "g",
  );
  const atoms = [...source.matchAll(rspackAtom)];
  if (atoms.length !== 1) throw new Error(`API quota Rspack selector count=${atoms.length}; expected=1`);
  const match = atoms[0];
  const getter = match[2];
  const data = match[4];
  const body = source.slice(match.index, match.index + 2400);
  const guard = body.indexOf(`${data}.rate_limit?.allowed!==!1`);
  if (guard < 0) throw new Error("API quota Rspack selector rate_limit.allowed guard missing");

  // Locate the saved-config module (["user-saved-config"]) and its exported descriptor atom.
  const cfgModRe = new RegExp(
    `(${id}):\\(function\\(${id},${id},${id}\\)\\{[^{}]*?let ${id}=\\["user-saved-config"\\],(${id})=\\(0,${id}\\.${id}\\)\\(${id}\\.${id},\\(\\{cwd:${id}=null,hostId:${id}\\}`,
    "g",
  );
  const cfgMods = [...source.matchAll(cfgModRe)];
  if (cfgMods.length !== 1) throw new Error(`API quota Rspack saved-config module count=${cfgMods.length}; expected=1`);
  const cfgModuleId = cfgMods[0][1];
  const cfgLocalVar = cfgMods[0][2];
  const cfgSlice = source.slice(cfgMods[0].index, cfgMods[0].index + 1600);
  const cfgExportMatch = new RegExp(`(${id}):${cfgLocalVar}\\b`).exec(cfgSlice.slice(cfgSlice.indexOf(".d(")));
  if (!cfgExportMatch) throw new Error("API quota Rspack saved-config export key missing");
  const cfgExportKey = cfgExportMatch[1];
  const resolvedConfigRef = { moduleId: cfgModuleId, exportKey: cfgExportKey };

  // Collect candidate chunk IDs from LocalConversationPage's Promise.all loader so the lazy
  // banner chunk can be pre-armed before reload.
  const bannerChunkIds = new Set();
  for (const pageMatch of source.matchAll(/\)\.LocalConversationPage\b/g)) {
    const start = source.lastIndexOf("Promise.all([", pageMatch.index);
    if (start >= 0 && pageMatch.index - start < 8000) {
      for (const eMatch of source.slice(start, pageMatch.index).matchAll(/\.e\((\d+)\)/g)) {
        bannerChunkIds.add(Number(eMatch[1]));
      }
    }
  }

  const offset = match.index + match[0].lastIndexOf("if(");
  const atomExpr = `globalThis.__codexSwitchWebpackRequire?.(${JSON.stringify(cfgModuleId)})?.[${JSON.stringify(cfgExportKey)}]`;
  return location("send_gate", offset, match.index + guard,
    configCondition(`((cfgAtom => cfgAtom ? ${getter}(cfgAtom, { hostId: "local", cwd: null }) : null)(${atomExpr}))`, data,
      `${data} = { ...${data}, rate_limit: { ...${data}.rate_limit, allowed: true } }; state.unlockCount += 1;`),
    { atom: match[1], getter, data, configRef: resolvedConfigRef, bannerUrls: [], bannerChunkIds: [...bannerChunkIds] });
})
