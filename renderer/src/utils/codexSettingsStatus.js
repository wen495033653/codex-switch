import { translate, translateRuntimeText } from '../i18n';

export function proxySaveFeedback(result) {
  const error = result?.remoteControl?.error;
  return {
    restartRequired: result?.restartRequired === true,
    message: error
      ? translate('{message} 远程控制代理更新失败：{error}', {
        message: translateRuntimeText(result.message),
        error: translateRuntimeText(error)
      })
      : result?.message,
    warning: Boolean(error),
  };
}
