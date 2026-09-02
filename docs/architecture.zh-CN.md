# 架构

[English](architecture.md)

## 进程边界

```text
React WebView
  | 固定的 Tauri 命令与脱敏事件
Tauri 启动层（非特权桌面进程）
  | 权限 0600 的 Unix Socket，版本化 /v1 API
控制器容器（restart=unless-stopped）
  | 通过 /var/run/docker.sock 调用 Docker API
Webtop 容器 / 共享 frpc / 隔离 Worker
  | 期望状态与检查点
SQLite WAL + 应用自有的绝对数据路径
```

WebView 不能提交命令、任意 Docker JSON、容器名称、破坏性操作使用的主机路径或
Shell 片段。Tauri 会校验范围有限的命令接口，并转发类型化契约。Unix Socket
本身是一条信任边界，因此控制器还会重复校验。

## 所有权与状态协调

控制器只识别带有全部必需 `com.cue.webtop-manager.*` 标签的资源。它绝不会把
镜像继承关系、容器名称前缀、Compose 标签或 `/config` 内容视为所有权证明。

控制器从 `/data/environments` 访问环境存储，但 Docker bind mount 的源必须是
主机路径。因此，桌面端通过 `WEBTOP_MANAGER_HOST_ENVIRONMENT_ROOT` 传入解析后的
Tauri 应用数据环境根目录；控制器使用内部路径进行文件系统和删除检查，仅在创建
受管 Webtop 容器时使用主机路径。

SQLite 记录期望状态和绝对路径。Docker 标签则提供状态协调和孤立资源报告所需的
外部身份。

模板在不扁平化镜像的前提下保持独立可迁移。控制器会把停止的受管环境提交为
多层镜像 `com.cue.webtop-manager/template:<uuid>`，并单独快照 `/config`。
Docker 的内容寻址层可以在本机共享，但模板标签不依赖官方源标签继续存在。导出
采用 Docker save/load 语义，因此镜像配置和所有必需的父层都会包含在包中。导入
清单里的外部来源仅用于提供信息；只有本机创建的子模板才会获得本地父级外键。

耗时模板任务以行记录保存在 SQLite 中。请求返回操作 UUID，界面轮询
`GET /v1/operations/{id}`。关闭 Tauri 不会终止控制器任务。如果控制器自身重启，
非终态记录会变为 `retryable`，控制器会在重新提供服务前清理自己产生的残留文件。
模板导入和导出还接受 `DELETE /v1/operations/{id}`。停止请求会让操作进入回滚；
只有在中间文件、镜像标签和未发布快照全部清理后，才会持久化 `cancelled` 状态。

## 稳定 API 接口

首个 API 命名空间为 `/v1`。错误只包含稳定的错误代码和安全的字符串参数。内部
Docker、SQLite、文件系统和命令输出仅在脱敏后写入本地日志，绝不会复制到前端
事件中。

首个代码里程碑已经实现以下路由：

- `GET /v1/health`
- `GET|POST /v1/environments`
- `POST /v1/environments/{id}/start|stop|restart`
- `DELETE /v1/environments/{id}`，需要输入准确名称确认
- `GET /v1/images/official` 和受允许列表约束的 `POST /v1/images/pull`
- `GET|PUT /v1/settings/server`，FRP Token 存储在 SQLite 之外
- `POST /v1/settings/server/token/recover`，仅在本地受保护 Token 缺失或失效时
  接受；SQLite 只保存其 SHA-256 指纹和恢复状态
- `GET /v1/frpc` 以及固定的 frpc `start|restart|stop|test` 操作
- `GET /v1/frps/setup`，仅在用户明确请求时返回包含秘密的配置命令
- `GET|POST /v1/templates`，以及模板预检、恢复、源检查、导出和依赖感知删除路由
- `POST /v1/template-imports/preflight|/v1/template-imports`，仅使用 UUID 暂存标识
- `GET /v1/operations/{id}`，用于持久化进度和类型化结果
- `DELETE /v1/operations/{id}`，用于协作式停止模板导入和导出
