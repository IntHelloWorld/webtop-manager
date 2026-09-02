# 参与贡献

[English](CONTRIBUTING.md)

Webtop Manager 是一款处于 alpha 阶段、对安全敏感的桌面应用。范围小而聚焦、带
测试并明确说明安全考量的变更最容易审查。

## 配置开发环境

受支持的开发环境是 Ubuntu 24.04、Linux x86_64 和本机 Docker Engine。请先按照
[开发指南](docs/development.zh-CN.md)完成配置，再运行：

```bash
./scripts/doctor.sh
./scripts/check.sh
cargo check --package webtop-manager
```

修改控制器或 Worker 后，还需要重新构建并校验内置 OCI 归档：

```bash
./scripts/package-controller.sh
zstd --test --quiet src-tauri/assets/controller-image.tar.zst
```

生成的归档有意被 Git 忽略，并由发布工作流重新构建。

## Pull Request

- 保持 WebView API 类型化并受允许列表约束；不要添加通用 Shell、Docker、路径
  删除或 URL 打开命令。
- 每个破坏性操作都必须保留应用所有权标签检查和规范路径检查。
- 不要在 API 值、SQLite 记录、日志、测试夹具或前端事件中存放密码、Token、
  私有主机路径或命令输出。
- 公网发布必须默认关闭，并以醒目的方式确认风险。
- 为可观察行为新增或更新 Rust 和前端测试。
- 当边界或验收缺口变化时，更新安全、架构和状态文档。
- 不要提交 `.wtmpl`、`.env*`、数据库、证书、安装包、控制器归档或应用数据文件。

请使用 Pull Request 模板，并确保所有适用检查均已完成。安全漏洞必须按照
[安全策略](SECURITY.zh-CN.md)私下报告，不要通过 Pull Request 或公开 Issue 报告。
