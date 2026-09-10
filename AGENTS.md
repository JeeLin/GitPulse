# GitPulse — 项目约定

## 技术栈
- 语言：Rust
- TUI：ratatui
- Git：git2-rs
- GitHub API：octocrab
- 异步：tokio
- 本地存储：SQLite（rusqlite）
- 依赖管理：mise

## 项目结构
```
GitPulse/
├── docs/
│   ├── PRODUCT.md          # 产品文档
│   ├── DEVELOPMENT.md      # 开发设计文档
│   └── milestones/         # 里程碑文档（.dev-flow/milestones/ 本地存储）
├── src/
│   ├── main.rs
│   ├── app.rs              # 应用入口
│   ├── ui/                 # TUI 界面
│   ├── git/                # Git 操作（git2-rs）
│   ├── github/             # GitHub API（octocrab）
│   ├── config/             # 配置管理
│   └── db/                 # SQLite 存储
├── Cargo.toml
├── mise.toml
└── .gitignore
```

## 代码规范
- 遵循 Rust 官方风格（rustfmt）
- 公共 API 必须有文档注释
- 错误处理使用 anyhow + thiserror
- 异步代码使用 tokio

## 测试命令
- 编译检查：`cargo check`
- Lint：`cargo clippy -- -D warnings`
- 测试：`cargo test`
- 覆盖率：`cargo llvm-cov`

## 质量门禁
| 检查项 | 命令 |
|--------|------|
| 编译检查 | `cargo check` |
| Lint 检查 | `cargo clippy -- -D warnings` |
| 测试 | `cargo test` |
| 覆盖率 | `cargo llvm-cov`（目标 90%） |

## 审查维度
- 功能完整性
- 错误处理
- 性能
- 安全性

## 设计审查配置
人工复核：可选
