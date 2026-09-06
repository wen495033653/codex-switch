# 默认 API 测试模型

- 2026-09-06：前端和 Rust 默认值统一为 `gpt-6-astra`；显式输入其它模型不变。
- 验证：Rust `cargo test api_test_model_tests` 1 项通过；前端实际调用 normalize 函数，缺省、空白、自定义模型 3 项通过。
- 验证层级：本地函数行为；未向账号 API 发送计费请求。
