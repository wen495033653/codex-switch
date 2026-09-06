export function proxySaveFeedback(result) {
  const error = result?.remoteControl?.error;
  return {
    restartRequired: result?.restartRequired === true,
    message: error ? `${result.message} 远程控制代理更新失败：${error}` : result?.message,
    warning: Boolean(error),
  };
}
