# 1.0 版本实现状态

[English](v1-status.md)

1.0 版本已经满足本仓库定义的发布验收范围。产品边界有意保持明确而有限：仅支持
Linux x86_64、本机 Docker Engine，以及由 Webtop Manager 新建的环境。

## 1.0 已验收能力

- 独立的 Tauri 2 + React/TypeScript/Vite + Rust 工作区。
- Docker 缺失、权限不足、守护进程异常和 Socket 全局可写诊断。
- 内置控制器镜像的构建/加载路径，以及加固的常驻容器。
- 权限为 `0600` 的 Unix Socket `/v1` API、版本化 SQLite Schema 和 WAL 状态。
- 类型化的环境创建、启动、停止、重启和删除操作。
- 保留字段检查、非特权策略、所有权标签和规范化删除路径边界。
- 通过密码文件注入秘密，不将密码写入 Docker 环境变量、SQLite、日志或前端事件。
- LinuxServer Webtop 官方镜像清单、本地镜像识别、允许列表拉取、实时进度、明确
  取消、控制器重启后续传和桌面端重新关联。
- 持久化 FRP 设置，Token 隔离存放在权限为 `0600` 的文件中。
- 一次性自动生成 FRP Token、基于指纹的丢失检测、受限的远端重新配对、服务器配置
  指南、共享 frpc 生命周期控制、认证连通性测试，以及按环境明确开启公网发布。
- 串行化远程端口分配；外部 FRP 客户端在并发竞争中抢占端口时可检测并自动重试。
- 主机路径正确的 `/config` bind mount，以及受允许列表约束的受管目录打开操作。
- 保留完整 `/config` 快照的独立多层模板，包括保守预检、元数据与摘要校验、来源
  关系和依赖感知删除。
- 版本化 `.wtmpl` 导出/导入，包括固定载荷、离线 Docker load、路径穿越防护、
  哈希/大小校验和仅使用 UUID 的原生暂存机制。
- 持久模板操作，包括有界脱敏输出、协作式取消、重启安全的终态和残留产物清理。
- 控制器升级：在中断前导入、备份受保护状态、拒绝较新 Schema、使用迁移状态运行
  候选版本、验证健康状态、原子切换容器名称，并在失败时恢复旧状态和控制器。
- 基于真实 Docker 的发布验收，覆盖离线模板往返、FRP 并发端口冲突、本地及公网
  HTTPS/TLS，以及 Docker Inspect、API 响应、SQLite、清单和控制器日志中的秘密
  泄漏检查。
- 带 SHA-256 校验和的 Linux x86_64 `.deb` 与 AppImage 发布包。
- 完整的简体中文和英文桌面界面、README 与项目文档。

## 明确不在 1.0 范围内

以下项目是 1.0 版本的产品非目标，不会阻塞发布：

- 修改不可变环境配置时使用安全重建事务。
- 在受允许列表约束的官方目录和应用自有模板接口之外，提供通用镜像生命周期操作。
- 允许用户配置 XDG 环境与快照根目录。

现有 Compose 项目和容器同样位于所有权边界之外：Webtop Manager 绝不会导入、
接管、重建或删除它们。

## 发布门禁

每次 1.0 版本发布都必须通过：

```bash
./scripts/check.sh
cargo check --package webtop-manager --locked
./scripts/check-release-version.sh v1.0.0
./scripts/package-controller.sh
zstd --test --quiet src-tauri/assets/controller-image.tar.zst
./scripts/test-packaged-controller.sh
./scripts/test-docker-acceptance.sh
```

基于 Docker 的测试套件会在 `main` 分支、手动触发 CI 和发布工作流创建安装包前
运行。它使用隔离的状态、Socket、FRP 容器和应用自有测试资源。
