(function locateApiQuota(source, url) {
  const id = "[A-Za-z_$][\\w$]*";
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
  const before = source.slice(0, offset).split("\n");
  const end = source.slice(0, match.index + guard).split("\n");
  const condition = `(() => {
    const state = globalThis.__codexSwitchApiQuotaState ??= { unlockCount: 0, lastProvider: null, lastError: null };
    try {
      const provider = ${getter}(${config}, { hostId: "local", cwd: null }).data?.model_provider ?? "openai";
      state.lastProvider = provider;
      if (provider === "api" && ${data}?.rate_limit != null) {
        ${data} = { ...${data}, rate_limit: { ...${data}.rate_limit, allowed: true } };
        state.unlockCount += 1;
      }
    } catch (error) {
      state.lastError = String(error?.stack ?? error);
    }
    return false;
  })()`;
  return {
    url,
    lineNumber: before.length - 1,
    columnNumber: before.at(-1).length,
    endLineNumber: end.length - 1,
    endColumnNumber: end.at(-1).length,
    atom: match[1],
    getter,
    data,
    config,
    condition,
  };
})
