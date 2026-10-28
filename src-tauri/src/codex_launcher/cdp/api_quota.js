(function locateRemoteControlQuota(source, url) {
  const id = "[A-Za-z_$][\\w$]*";
  const configCondition = (read, data, action, host = '"local"') => `(() => {
    const state = globalThis.__codexSwitchApiQuotaState ??= { enabled: true, unlockCount: 0, hiddenBannerCount: 0, lastProvider: null, lastError: null };
    try {
      if (!state.enabled || ${host} !== "local") return false;
      const config = ${read}.data;
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

  if (url.includes("/assets/composer-content-")) {
    // The local composer derives this banner model from the official quota
    // snapshot. Remove only that render-local snapshot, never the shared query.
    const result = new RegExp(`imageGenerationLimit:${id},isUsageBannerEnabled:${id},onDismissImageGenerationLimit:${id},rateLimit:(${id}),rateLimitWarningThreshold:${id},sendBlocked:`, "g");
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
})
