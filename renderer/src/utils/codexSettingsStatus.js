export function remoteControlPrerequisites({ subscriptionMode, accountId, accountPresent, authInvalid }) {
  const reasons = [];
  if (subscriptionMode) reasons.push('请先切换到 API 模式');
  if (!accountPresent) {
    reasons.push(accountId ? '已选控制账号不存在，请重新选择' : '请先选择远程控制账号');
  } else if (authInvalid) {
    reasons.push('控制账号登录已过期，请重新登录');
  }
  return reasons;
}

export function proxySaveFeedback(result) {
  const error = result?.remoteControl?.error;
  return {
    restartRequired: result?.restartRequired === true,
    message: error ? `${result.message} 远程控制代理更新失败：${error}` : result?.message,
    warning: Boolean(error),
  };
}
