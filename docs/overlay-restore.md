# 备份迁移恢复

`overlay-restore` 是独立恢复后端，`luci-app-overlay-restore` 提供 LuCI 页面。
`router/restore_overlay.sh` 调用同一后端，默认软件列表继承原脚本。
目标系统是使用 APK 的 OpenWrt 25.12，仓库的软件源和构建目标为 `x86_64`。

后端使用 Rust 1.99.0 稳定版、Rust 2024 edition，编译为
`x86_64-unknown-linux-musl` 静态 ELF 程序。路由器无需安装 Python 或 Rust 工具链。
LuCI 保持 JavaScript 页面，通过 rpcd/ubus 调用 `/usr/sbin/overlay-restore`。
Rust 工具链版本固定在 `rust-toolchain.toml`，依赖版本和校验值固定在
`packages/overlay-restore/src/Cargo.lock`；当前只构建 x86_64。
程序使用 x86-64 基线指令集，不使用开发机的 `target-cpu=native`。
原后端的 JSON 任务记录、UCI 配置和 CLI/RPC 接口继续使用相同格式。

## 安装与使用

发布本仓库的新软件包后，在已配置 myfeed 的路由器上执行：

```sh
apk update
apk add luci-app-overlay-restore
```

LuCI 入口为「系统 → 备份迁移恢复」。修改选项后点击「上传并检查备份」，
页面会先提交 `overlay_restore` 配置，再上传文件并生成计划。
检查待恢复文件、软件列表、网络地址和凭据选项后，点击「确认恢复计划 → 执行恢复」。
默认自动重启，重启后重新登录同一页面查看软件安装结果。

命令行也分为检查和确认两个步骤：

```sh
overlay-restore inspect /tmp/overlay_backup.tar.gz
# 用上一步返回的 id 替换 TASK_ID。
overlay-restore apply TASK_ID --confirm TASK_ID
overlay-restore status TASK_ID
overlay-restore retry TASK_ID
```

`apply --no-reboot` 只迁移文件，需要手动重启后才进入软件恢复阶段。
兼容入口 `sh router/restore_overlay.sh BACKUP` 会显示 JSON 计划并要求输入 `YES`；
`--inspect` 只检查。旧的独立脚本已替换，必须先安装恢复后端。

## 迁移范围

这是一份配置迁移计划，不是整盘还原。它通过挂载后的 `/` 写入文件，
不会删除正在使用的 `/overlay/upper` 或 `/overlay/work`。
新系统中没有出现在备份里的文件仍然保留。
普通文件采用备份中的权限位；覆盖现有文件时保留当前 UID/GID，新增文件由 root 拥有。
不会从旧固件复制 UID/GID 或 setuid/setgid 权限。

支持 gzip tar 中的 `overlay/upper/`、`upper/` 和普通 sysupgrade `etc/config/` 布局。
可以读取旧 APK `lib/apk/db/installed`、新 APK `lib/apk/packages/*.list`
和 opkg 文件清单，用来跳过旧软件程序文件。

| 内容 | 处理方式 |
| --- | --- |
| `/etc` 中的配置、`/root` 自定义普通文件 | 按选项迁移；当前工具、系统标识等路径受保护 |
| `/www`、`/opt`、`/srv` 自定义普通文件 | 跳过软件包所有的文件后迁移 |
| `/usr/bin`、`/usr/sbin`、`/usr/share` 自定义文件 | 仅有软件文件清单时迁移未归属于软件包的文件 |
| 内核模块、APK/opkg 数据库、软件源、公钥、LuCI 运行文件、init 脚本 | 保留当前系统版本 |
| 恢复后端、LuCI 页面、RPC/ACL、`/etc/config/overlay_restore`、任务目录 | 保留当前系统版本 |
| 软链接、硬链接、设备节点、whiteout、运行缓存 | 跳过；whiteout 不会删除当前文件 |
| `/mnt`、`/media` 等外接存储内容 | 不迁移，服务修复使用当前已经存在的目录 |

默认保留当前 fstab 中目标为 `/` 或 `/overlay` 的 extroot 条目，
备份中的其它挂载条目可以恢复。取消此选项会使用备份的 fstab。
账号和网络的默认行为与原脚本一致：恢复备份凭据，恢复备份网络。
可分别选择保留当前凭据、保留当前 network/dhcp/firewall。
跨设备时需要自行核对接口名、网段、磁盘 UUID 和服务配置。

备份缺少软件清单时会跳过不能确定用途的 `/usr` 程序文件；
需要的自定义程序可以另行迁移。旧固件软件二进制不会用来替代当前软件源版本。

## 任务与失败恢复

任务目录为 `/etc/overlay-restore/jobs/TASK_ID/`，目录权限 `0700`，
配置、状态和日志通常为 `0600`。目录内包含冻结后的选项、文件 SHA256、
恢复计划、待写入文件、写入前的原始文件以及软件执行记录。
日志可通过 LuCI 或 `overlay-restore status TASK_ID` 查看，页面显示末尾 40 KB。
原始文件和任务记录会保留，便于人工核对；目前没有自动清理历史任务。

流程为：

1. 校验压缩包 CRC、布局、路径、重复条目、展开大小和剩余空间，生成计划。
2. 用户确认后校验文件摘要，把待恢复文件写入持久存储。
3. 如果选择了缺失的 SmartDNS/Nikki，先从当前 myfeed 准备它们，避免备份 DNS/代理配置在重启后缺少运行程序。
4. 逐个保存原始文件、原子替换目标文件、记录进度；发生写入错误时尝试恢复原始文件。
5. 等待实际重启，在新一次启动中安装软件、修复主题和服务。

检查阶段不更改当前配置文件；原始压缩包和临时展开文件位于 `/tmp`。
检查后尚未确认的任务如果遇到重启，需要重新上传检查。
确认后的文件与进度进入持久存储，后台 worker 由 procd 管理，
关闭浏览器或 SSH 不会中断已确认的恢复任务。

网络软件准备失败时尚未迁移配置，可以重试。软件安装失败时已迁移的配置会保留，
重试只处理软件及服务步骤，不会再次覆盖用户后来修改的配置。
失败的软件阶段会在后续启动时自动重试，最多三次；也可在页面上手动重试。
配置写入失败的任务需先排除存储或目标路径问题，再重新上传检查。
多文件恢复不是一次不可分割的事务，突然断电、存储错误和恢复网络后的可达性
仍需要结合任务日志检查。

myfeed 在执行期间临时加 `@myfeed` 标签。原软件源内容、原有 world 标签会先保存，
执行结束或后台恢复时会复原，并去掉本任务新加的 world 标签。
不会使用 `--force-broken-world`。必需软件失败会保留失败状态；可选软件失败单独显示警告。
移除列表只接受 LuCI 应用、翻译、协议页面和主题，并禁止移除恢复工具及基础界面。

## 配置与服务

配置文件为 `/etc/config/overlay_restore`，主 section 为 `main`、类型为 `restore`。
网页可编辑软件安装/移除列表、备份大小限制、extroot/网络/凭据选项和 IPTV 参数。
现有 `00-myfeed.list` 优先于默认源，源地址会在检查时冻结。
默认上传上限 256 MiB、展开上限 2048 MiB，实际可处理大小还受 `/tmp` 和 overlay 空间限制。
确认阶段会为待写入文件、原始文件和原子替换预留空间。

CLI 保留原脚本的 `RESTORE_INSTALL_PACKAGES`、`RESTORE_MYFEED_INSTALL_PACKAGES`、
`RESTORE_MYFEED_OPTIONAL_INSTALL_PACKAGES`、`RESTORE_REMOVE_PREINSTALLED_LUCI_PACKAGES`
及 IPTV 等环境变量入口，具体映射见 `packages/overlay-restore/src/src/settings.rs`。
`RESTORE_KEEP_EXTROOT=1` 保留旧变量的含义：使用备份的 extroot；网页选项采用直接命名。
环境变量只影响这一次 CLI 检查，RPC 使用保存的 UCI 配置。

服务修复会注册实际安装的主题并选择可用主题，按原配置重启 SmartDNS/Homebox。
IPTV Refresh 安装成功且当前数据目录存在时，才会配置并启用它，
写入刷新端点、允许网段、令牌和 Home Assistant 配置。
不会自动创建缺失的外接磁盘根目录。Home Assistant 目录缺失时不写入它。
令牌留空时生成随机令牌并存入任务私有选项及 `/etc/iptv-refresh/token`。

RPC 仅暴露 `prepare/list/status/apply/retry`；上传只允许固定临时文件名。
ACL 的只读部分只允许读取任务与配置，写入部分允许恢复和修改本应用配置，
不授予任意命令、任意文件或其它 UCI 配置的权限。

## 只用 WSL 开发和验证

下面的 QEMU/浏览器步骤使用本地 `tests/` 和三个准备脚本
`scripts/setup-restore-dev.sh`、`scripts/fetch-restore-test-tools.sh`、
`scripts/setup-restore-browser.sh`。它们和测试产物已被 `.gitignore` 排除，
只保留在当前工作区，新克隆的仓库不包含这些辅助文件。
仓库 CI 使用的 Rust 回归测试和 `scripts/test-restore-rust.sh` 继续纳入版本管理。

Windows 上安装 WSL2 Ubuntu 后，在专用 Ubuntu 环境执行下列命令。
SDK、QEMU 和浏览器都运行在 WSL；QEMU 里面运行实际的 OpenWrt。
无需安装 Hyper-V/VirtualBox，也不会把 Ubuntu 本身改造成路由器。

```sh
cd /mnt/d/Openwrt_packages
sudo bash scripts/setup-restore-dev.sh
sudo bash scripts/fetch-restore-test-tools.sh
sudo env SDK_DIR=/opt/overlay-restore-dev/sdk \
  RESTORE_RUST_ROOT=/opt/overlay-restore-dev/rust \
  bash scripts/build-local-apks.sh bin/packages/x86_64/myfeed
sudo env RESTORE_RUST_ROOT=/opt/overlay-restore-dev/rust \
  bash scripts/test-restore-rust.sh
sudo python3 tests/qemu_restore_smoke.py --keep-running
```

本地 APK 输出在 `bin/packages/x86_64/myfeed/`。源码可在 Linux 用户目录构建，
SDK 建议位于 WSL 的 Linux 文件系统，避免大量小文件在 `/mnt/d` 上拖慢构建。
没有设置 `SDK_DIR` 时，构建脚本使用仓库 CI 相同的 Docker SDK 镜像。
Rust 构建使用固定工具链、`--locked` 依赖以及 SDK 的 x86_64 musl 链接器；
依赖先下载，再在 SDK 编译阶段通过 `--offline` 使用缓存。
回归测试由 `cargo test` 执行，并运行 `cargo fmt` 和 Clippy；Python 只用于
开发机上的 QEMU/浏览器测试，不会作为恢复工具的路由器依赖。
`build-feed` 工作流会把两个本地 APK 加入软件源并重新签名索引；本地构建不会发布它们。

测试使用校验过 SHA256 的 OpenWrt 25.12.0 x86_64 squashfs 镜像，
创建独立 qcow2 磁盘。虚拟磁盘放在 WSL 的 `/opt/overlay-restore-dev/test/disks/`，
避免 Windows 挂载目录的小文件 I/O 开销。SSH 和 HTTP 只转发到本机：

- 浏览器：`http://localhost:18080/cgi-bin/luci/admin/system/overlay_restore`
- SSH：端口 `22222`，测试密钥在 WSL `/opt/overlay-restore-dev/test/id_ed25519`
- 测试数据与报告：`.restore-test-cache/`，已被 git 忽略

测试检查 APK 安装、检查阶段不写配置、旧程序跳过、实际重启、跨重启续跑、
curl 安装，以及只读/受限管理账号的 RPC 权限。虚拟机没有连接物理路由器。
QEMU 优先使用 `/dev/kvm`，不可用时回退 TCG。
可用 `RESTORE_QEMU_ACCEL=tcg` 强制 CPU 模拟。
保留测试机运行后，可以继续执行实际浏览器测试：

```sh
sudo bash scripts/setup-restore-browser.sh
sudo env PLAYWRIGHT_BROWSERS_PATH=/opt/overlay-restore-dev/browsers \
  /opt/overlay-restore-dev/browser-venv/bin/python tests/qemu_restore_browser.py
```

浏览器测试核对表单提交、空软件列表、上传、预览、确认、保留凭据和手动重启选项，
同时保存截图。测试机 root 初始密码为空，只用于回环端口上的隔离测试。
加 `--with-myfeed` 会通过页面选择 Bandix，并在手动重启后验证真实 myfeed
签名索引、APK 安装及临时源/world 标签清理，需要访问当前在线 myfeed。
浏览器测试故意停在「等待手动重启」，方便检查页面；需要结束完整流程时可用
测试密钥 SSH 执行 `reboot`，随后查看任务状态。
停止保留的虚拟机可以执行 `sudo python3 tests/qemu_restore_smoke.py --stop`，
脚本会核对 PID、QEMU 程序和工作区测试磁盘路径。重新启动使用全新测试磁盘，
不要同时占用相同转发端口。

WSL/QEMU 可验证实际 OpenWrt 的 squashfs、overlay、APK、rpcd、procd、LuCI 和重启流程。
物理路由器的 extroot 磁盘、实际网口/拨号、组播 IPTV 与 Home Assistant 联动，
仍需使用相应硬件和数据目录进一步验证。
