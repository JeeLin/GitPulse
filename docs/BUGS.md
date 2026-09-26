# 缺陷池

| 提出版本 | 优先级 | 标题 | 来源 | 描述 |
|----------|--------|------|------|------|
| v0.2.0 | 🔴 | 批量 pull 不复用系统凭据、交互式询问账户 | 用户反馈 | 软件启动时即应加载并复用系统凭据配置（git credential helper / gh 等），远端操作全程非交互；现状是批量 pull 中出现「Username for 'https://github.com':」，TUI 无法应答导致卡住 |
| v0.2.0 | 🟡 | TOML/serde 无法序列化非 UTF-8 路径，非 UTF-8 扫描目录不可持久化 | 步骤5代码审查（fix-28 上报） | serde 对含非 UTF-8 字节的 `PathBuf` 直接报错（`path contains invalid UTF-8 characters`），`Config::save` 失败并回滚 → 该类目录经 picker 可选中但保存必失败；需 config 层改用 OsString 友好编码（如 toml_edit 原始字符串或字节转义）才能支持 |
