# 开发环境与启动指南

本文档适用于项目当前支持的平台：Ubuntu 24.04、Linux x86_64 和本机
Docker Engine。项目不会自动安装 Docker、修改 Docker 用户组或更改
`/var/run/docker.sock` 权限。

## 当前固定版本

| 工具 | 版本 |
| --- | --- |
| Rust | 1.88.0 |
| Node.js | 22.23.2 |
| pnpm | 10.4.1 |
| Tauri | 由 `Cargo.lock` 固定 |
| 前端依赖 | 由 `pnpm-lock.yaml` 固定 |

Node.js 二进制安装到
`~/.local/share/webtop-manager-dev/node-v22.23.2-linux-x64`，并通过
`~/.local/bin` 暴露命令。下载文件会使用 Node.js 官方 SHA-256 校验。
Rust 使用官方 rustup 安装到默认的 `~/.rustup` 和 `~/.cargo`。

## 一次性配置

在项目根目录执行：

```bash
./scripts/setup-dev.sh --install-system-deps
```

该命令的系统依赖部分会调用 `sudo apt-get`，需要你输入本机管理员密码。
脚本随后会幂等地配置 Rust、Node.js、pnpm，并执行
`pnpm install --frozen-lockfile`。如果不希望脚本调用 sudo，可省略参数；它会列出
缺少的软件包而不修改系统。

新终端会通过 `~/.profile` 自动获得 `~/.local/bin` 和 `~/.cargo/bin`。
当前终端也可以立即加载：

```bash
source ./scripts/dev-env.sh
```

## 环境自检

```bash
./scripts/doctor.sh
```

自检覆盖 Node.js、pnpm、Rust、Cargo、Docker CLI、Docker Daemon、C 编译器、
WebKitGTK 4.1、zstd 和控制器 OCI。没有控制器 OCI 只会产生警告，因为诊断模式
仍可独立运行。

## 启动开发模式

只启动桌面 UI 和 Docker 诊断：

```bash
./scripts/dev.sh
```

`pnpm tauri dev` 会依据 `src-tauri/tauri.conf.json` 自动启动 Vite 开发服务器，
不需要另开终端运行 `pnpm dev`。React/CSS 修改支持热更新，Rust/Tauri 修改会触发
桌面进程重新编译。

要同时构建并嵌入控制器 OCI：

```bash
./scripts/dev.sh --with-controller
```

首次执行需要构建静态 Rust 控制器与 Worker、保存 Docker 镜像并进行 zstd 压缩，
耗时会明显长于普通启动。脚本只创建 `com.cue.webtop-manager/controller:0.1.0`
以及项目资源文件，不会导入或修改已有 Compose 环境。

## 常用验证命令

```bash
./scripts/check.sh
```

该脚本依次执行 Rust 格式检查、核心 Rust 测试、锁文件安装、前端测试和生产构建。
完整 Tauri 检查可单独执行：

```bash
cargo check --package webtop-manager
```

## 常见问题

### `cargo`、`node` 或 `pnpm` 找不到

打开一个新终端，或执行：

```bash
source ./scripts/dev-env.sh
```

### 提示缺少 `webkit2gtk-4.1`

系统依赖尚未安装。执行：

```bash
./scripts/setup-dev.sh --install-system-deps
```

### APT 依赖版本不匹配

如果安装时出现 `Depends: ... (= 较旧版本) but 较新版本 is to be installed`，先检查
Ubuntu 的 updates 软件源是否被禁用：

```bash
apt-mark showhold
apt-cache policy libgtk-3-0t64 libgtk-3-dev zlib1g zlib1g-dev
```

Ubuntu 24.04 的 `/etc/apt/sources.list.d/ubuntu.sources` 应当为主 Ubuntu 镜像启用
以下 suites：

```text
Suites: noble noble-updates noble-backports
```

使用 `sudoedit /etc/apt/sources.list.d/ubuntu.sources` 修改该行，不要降级已经安装的
运行库。保存后刷新索引并重试：

```bash
sudo apt-get update
./scripts/setup-dev.sh --install-system-deps
```

安装脚本会在调用 APT 前检查 `noble-updates`，缺失时给出明确提示，不会自动修改
系统软件源。

### 界面提示控制器镜像缺失

这是未生成 OCI 时的预期诊断状态。需要环境管理功能时执行：

```bash
./scripts/package-controller.sh
./scripts/dev.sh
```

### Docker 权限不足

应用只负责诊断，不会自动执行 `usermod` 或修改 Socket 权限。请由系统管理员根据
本机安全策略配置 Docker 访问权限，然后重新登录。

### 启动时报 `ENOSPC: System limit for number of file watchers reached`

项目的 Vite 配置已排除 Rust `target/` 构建目录，以避免为编译产物创建大量文件
监听器。如果其他开发工具仍耗尽系统监听器，请先关闭不再使用的开发服务器或 IDE
工作区，再重新执行 `./scripts/dev.sh`；项目本身不自动修改系统级 inotify 限制。
