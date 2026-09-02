# 安全模型

[English](security.md)

Docker Socket 访问权限等同于 root。即使控制器容器使用只读根文件系统、不公开
端口、启用 `no-new-privileges`、不保留 Linux capabilities，并且只挂载最少的
目录，也必须将控制器视为高权限本地服务。

## 强制安全约束

- WebView 无法调用通用 Shell、exec、Docker、路径删除或 URL 打开命令。
- 拒绝 `privileged=true`、保留的 `/config` 挂载和包含秘密的环境变量键。
- 只有通过明确标注危险性的开关才能挂载 Docker Socket。
- Webtop 密码随机生成，使用 `FILE__PASSWORD` 从权限为 `0600` 的文件挂载。
- 删除官方镜像时只接受文档规定的允许列表、绝不强制删除；如果镜像拉取正在进行，
  或任何运行中/已停止容器仍使用该镜像，则拒绝删除。缓存清理和带标签镜像删除会
  与拉取注册串行执行，避免删除与下载发生竞争。
- SQLite 记录、事件和普通 API 值均不包含秘密。两条专用读取路径属于明确的例外：
  用户打开配置指南后，`/v1/frps/setup` 会返回包含 FRP Token、可复制的服务器
  命令；`/v1/environments/{id}/credentials` 只为当前已发布到公网的环境读取受
  保护的 Webtop 密码。环境卡片默认遮挡该密码。
- FRP Token 只生成一次，SQLite 仅记录其 SHA-256 指纹。秘密文件缺失或被替换时会
  暂停 frpc 自动启动；原 Token 健康时恢复接口会拒绝调用，恢复凭据只有在认证测试
  成功后才正式生效。
- 删除应用数据需要输入准确名称确认，且规范化路径必须严格位于环境根目录之下。
- 用户提供的外部挂载目录绝不会被自动删除。
- 打开环境数据目录时，仅接受请求 UUID 对应的受管 `/config` 挂载；控制器会
  规范化路径，并验证其仍位于应用环境根目录之下。只有在修复主机路径前创建的
  环境才能使用准确匹配的旧 `/data/environments` 布局。
- 快照先写入权限为 `0600` 的临时文件，通过 SHA-256 校验后再原子重命名。
- 快照 Worker 只接受绝对路径，跳过 Socket、FIFO 和设备，并拒绝归档路径穿越。
- 模板由一次 Docker commit 和单独的完整 `/config` `tar.zst` 组成。commit 会
  保留不可变源镜像的启动元数据，同时移除实例身份和疑似秘密的默认环境变量。
  挂载的 `/config` 数据绝不会被假定为镜像的一部分。
- 模板操作输出只允许一组有界、由控制器生成且不包含路径的状态行。密码、Token、
  任意主机路径和容器命令输出绝不会复制到界面日志。
- 模板导入/导出路径不会跨越 WebView 边界。原生文件对话框在 Tauri 后端异步运行，
  后台线程以字节为单位报告复制进度并使用 `0600` 权限，传给 `/v1` 的只有 UUID
  暂存标识。控制器在专用暂存根目录下解析这些 UUID。
- 发布导入快照时，会先复制到快照文件系统内权限为 `0600` 的临时文件，再原子
  重命名，因此即使暂存和快照挂载位于不同文件系统，也不依赖跨文件系统重命名。
- 取消模板导入/导出采用协作式机制。只有在残留包、解压目录、未发布快照和临时
  导入镜像标签均已清理后，控制器才会报告 `cancelled`。原生桌面复制同样会在
  返回前删除未完成的目标文件。
- `.wtmpl` 导入只允许 `manifest.json`、`payload/image.tar.zst` 和
  `payload/config.tar.zst`。校验内容包括 Schema、Linux/amd64 平台、路径安全、
  大小、SHA-256、单个 Docker 镜像及预期内部标签，完成后才执行 Docker load。
  保存镜像的配置会改写为唯一的暂存标签和本机所有权标签，防止包覆盖现有应用
  镜像或冒充其身份。
- 模板包和快照没有加密，可能包含 `/config` 中的 SSH 密钥、浏览器资料、云凭据和
  其他敏感数据；保存、导入和导出流程均要求显示明确警告。
- 镜像拉取、缓存清理、commit、导入、导出和自有镜像删除共享控制器端资源锁。
  当某个环境正在生成快照或恢复时，其生命周期操作和删除请求会被拒绝。

## 公网暴露警告

Webtop 在 3001 端口上使用带自签名证书的 HTTPS。其当前官方文档指出，可选的内置
Basic Authentication 仅适用于可信本地网络，并建议在公网暴露时使用可靠的反向
代理。产品规范有意将 v1 限制为个人使用的 TCP 转发，不提供这一更强的认证层。
界面会持续显示该风险；公网发布默认关闭，且创建环境时必须明确开启。已发布的
环境卡片会显示分配的 URL，以及共享 frpc 客户端是否已连接。

参考资料：[LinuxServer Webtop](https://docs.linuxserver.io/images/docker-webtop/)、
[Tauri capabilities](https://v2.tauri.app/security/capabilities/) 和
[frp tokenSource](https://gofrp.org/en/docs/features/common/authentication/)。
