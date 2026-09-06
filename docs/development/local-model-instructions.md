# 本地模型指令文件

- 2026-09-06：优先使用 Codex home 下已有的 gpt-unrestricted.md，不再在启动/开关时覆盖。安装包文件只初始化缺失的文件，create_new 阻止并发覆盖。
- 本机 config 已指向用户当前 MD，本次没有改动 config.toml 或 MD 内容。
- 验证：cargo test model_instructions::tests，5 项通过；覆盖原始换行/中文保留、不读取安装包、首次初始化、重复初始化不覆盖、目录/资源缺失错误。
- 验证层级：临时目录实际文件读写，非真实 Codex 重启。
