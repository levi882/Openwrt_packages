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

LuCI 入口为「系统 → 备份迁移恢复」。电脑上的备份可点击「上传电脑上的备份并检查」，
页面会先提交 `overlay_restore` 配置，再上传文件并生成计划。
备份已在路由器上时，可通过 QuickFile 浏览或填写 `/mnt/.../backup.tar.gz` 等绝对路径，
然后点击「检查路由器上的备份」。支持 `.tar.gz` / `.tgz`，检查前同样保存恢复设置，
后端会将备份复制到任务临时目录，保留磁盘上的原文件；两种方式使用相同的大小和空间限制。
文件浏览统一使用 QuickFile。点击「打开 QuickFile 选择备份」，
在弹窗中双击目录进入，再点击 `.tar.gz` / `.tgz` 备份的文件名、文件行或勾选框，
即可自动回填完整路径并关闭弹窗，列表和网格视图均支持，无需复制路径。
再点击「检查路由器上的备份」生成计划。回填路径不会直接执行恢复。
QuickFile 嵌入入口需要路由器已安装并运行 `luci-app-quickfile`，且其独立页面能正常打开；
未安装或不可用时，页面会提示，也可直接填写备份路径。
也可以点击页面底部的「保存」或「保存并应用」独立保存恢复设置，供后续网页或 SSH 检查使用。
「重置」撤销表单中尚未保存的修改，回到最近保存的设置。
保存设置不会执行恢复或重启；已有恢复计划使用检查时的设置，修改后需要重新检查备份。
检查待恢复文件、软件列表、网络地址和凭据选项后，点击「确认恢复计划 → 执行恢复」。
默认自动重启，重启后重新登录同一页面查看软件安装结果。

命令行也分为检查和确认两个步骤：

```sh
overlay-restore inspect /tmp/overlay_backup.tar.gz
# 用上一步返回的 id 替换 TASK_ID。
overlay-restore apply TASK_ID --confirm TASK_ID
overlay-restore status TASK_ID
overlay-restore retry TASK_ID
overlay-restore usage
# 清理所有已完成和检查失败的任务资料（含日志、原件及暂存文件）。
overlay-restore cleanup
# 单独删除已结束任务，或取消不再使用的待确认任务。
overlay-restore remove TASK_ID --confirm TASK_ID
```

普通迁移模式下，`apply --no-reboot` 只迁移文件，需要手动重启后才进入软件恢复阶段。
兼容入口 `sh router/restore_overlay.sh BACKUP` 会显示 JSON 计划并要求输入 `YES`；
`--inspect` 只检查。旧的独立脚本已替换，必须先安装恢复后端。

## 外接磁盘本地软件源

后端 0.2.0-r13、页面 0.2.0-r19 起，可在恢复页面的「软件包」中选择
「恢复软件来源」为「外接磁盘本地缓存」，并在「本地软件源目录」选择
外接磁盘上的目录，例如 `/mnt/sda1/restore-feed`，然后点击页面下方
「准备 / 更新本地软件源」。下载在后台进行，关闭页面不会中断。
准备完成后再升级固件或检查恢复备份。磁盘需要在重启后自动挂载到同一路径；
不能把源放在临时目录、将被重建的 upper/work，或未挂载磁盘的空目录中。

准备阶段从当前官方源和 CF myfeed 下载所选必需软件、恢复后端、页面及全部依赖，
保留原始签名索引及 APK 的签名校验；不会安装软件或更改系统的软件源和 world。
可选软件不能下载时单独提示，已成功准备的必需软件仍可使用。
当前支持 OpenWrt 25.12 x86_64 的显式 HTTPS `packages.adb` 源配置。

选择本地恢复后，保留配置升级的引导安装、恢复前 DNS/代理准备、干净 overlay 准备、
重启后的软件安装都通过 APK `--no-network` 读取本地缓存。
本地源不完整、签名无效或磁盘不可用时报告失败，不会静默回退到 CF。
恢复成功后，普通的软件安装和更新继续使用原来的 CF 和官方在线源，无需手动切换。
迁移 fstab 时会保留承载本地源的当前磁盘挂载项，避免备份中的旧挂载配置让重启后找不到缓存。
把「恢复软件来源」改为「CF / 在线软件源」并重新检查备份，即可在线恢复；
本地目录和已有缓存可以保留。旧配置只有目录、没有来源选项时仍按本地恢复处理。

「恢复后同步本地缓存」默认每周一次，也可选择每天一次或只手动同步。
首次本地恢复的软件安装成功后，才启用自动同步；首次缓存仍需手动准备。
同步从当前 CF myfeed 和官方源刷新所选软件、恢复工具及完整依赖，
只下载缓存，不安装或升级已安装软件，不改动系统软件源或 world。
恢复任务进行中或等待重试时暂停自动同步，已有恢复计划继续使用检查时固定的快照。
关闭页面、路由器重启和清理已完成的任务后，同步计划仍保留。
页面显示同步状态和下次时间；到期时由后台服务检查（最多约一分钟延迟）。
如果到期时路由器未开机，开机后补同步；失败保留上一份可用缓存，约一小时后重试。
更改同步周期或关闭自动同步，保存后生效；更换本地目录需要完成该目录的本地恢复后再启用。

检查备份时会固定使用的本地快照，后来更新本地源不会改变已有任务。
更改必需软件列表或 myfeed 地址后，需要重新准备本地源；APK 会根据目标固件解析依赖，
缓存不保证适用于其它固件系列、架构或内核版本。新快照完整下载后才替换 `current`，
准备失败保留上一份缓存；旧快照与下载失败的目录不会自动删除。
旧任务仍可能引用它们，确认不再使用后再通过文件管理器清理。

命令行入口：

```sh
uci set overlay_restore.main.local_feed_dir='/mnt/sda1/restore-feed'
uci set overlay_restore.main.restore_source='local'
uci set overlay_restore.main.local_feed_sync='weekly'
uci commit overlay_restore
overlay-restore prepare-local-feed
overlay-restore local-feed-status
# 等待 status 为 ready 后，再 inspect / apply 恢复备份。
```

CLI 检查还支持一次性的 `RESTORE_LOCAL_FEED_DIR`、`RESTORE_SOURCE=local` 环境变量；
自动同步读取已保存的 UCI 设置。
页面的本地源状态查询属于只读权限，准备操作仅授予本应用的写入权限。

## 保留配置升级后自动安装恢复工具

后端 0.2.0-r11、页面 0.2.0-r17 起，默认开启「保留配置升级后自动装回恢复工具」。
先安装或更新到这两个版本；旧版本没有开机引导，不能在升级后自动补装。
页面关闭该选项并保存后，后续启动会跳过自动安装。

1. 在固件的 OTA 或刷写页面选择「保留配置」，然后升级。
2. 首次启动保留原来的网络设置，引导在后台等待软件源可用。
3. 缺少恢复后端或页面时，从已签名的 myfeed 安装它们。
4. 安装完成后进入「系统 → 备份迁移恢复」，选择备份、检查并确认恢复计划。
   需要重建外部 overlay 时，再在该计划中选择分区并确认执行。

引导独立列入 sysupgrade 的保留清单，包含安装脚本、开机入口、恢复工具配置、
myfeed 地址和随包提供的签名公钥。现有工具完整时不会重装。
仅支持使用本仓库 myfeed 的 OpenWrt 25.12 x86_64；普通固件升级仍会丢失
固件未预装的其他软件，它们需要通过恢复计划重新安装。
选择「不保留配置」时，引导也不会保留，仍需先安装恢复工具或使用预装它的固件。

每次启动最多尝试 10 次，重试间隔 30 秒，各网络请求超时 20 秒；通过 procd 后台运行，
不阻塞开机。安装前先模拟 APK 事务，若需要升级、降级或移除其他已安装软件则停止。
不会绕过软件源签名校验，也不会自动确认恢复、清理 overlay 或重启。
安装日志保留最新 64 KiB，可在 SSH 中查看状态和手动重试：

~~~sh
/etc/overlay-restore-bootstrap/run status
tail -n 80 /etc/overlay-restore-bootstrap/boot.log
/etc/init.d/overlay-restore-bootstrap restart
~~~

## 迁移范围

这是一份配置迁移计划，不是整盘还原。默认通过挂载后的 `/` 写入文件，
新系统中没有出现在备份里的文件仍然保留。
可选的「恢复前重建干净 overlay」会准备新的 upper/work，停止服务并进入 RAM 后才切换，
旧环境保留供回退；两种模式都不删除正在使用的 upper/work。
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
页面显示最近 10 个任务，磁盘最多保存 20 个任务；达到上限后停止接受新的备份检查，
需要先手动清理已结束任务，或取消不再使用的待确认任务。升级前已有的超额任务会保留。
日志可通过 LuCI 或 `overlay-restore status TASK_ID` 查看，页面显示末尾约 40 KB。
每个任务日志最多保留 1 MiB 的最新内容，超出时移除较早内容；后台启动时也会整理旧的超大日志。
「查看占用」按需统计任务文件和临时检查文件的大小，并显示任务所在磁盘的剩余空间。
常规状态轮询不扫描任务文件，避免因大量文件拖慢页面。

「清理已结束的任务」仅删除已完成（含警告）和检查失败的任务资料。
选中等待确认的任务后，可以使用「取消并删除任务」释放其临时检查文件。
正在检查、迁移、等待重启或安装软件的任务，以及恢复失败后尚未完成的任务会保留。
删除任务会同时删除它的日志、暂存文件和覆盖前保存的原件；当前配置和磁盘上的原始备份文件不会删除。
清理前请先保存仍需排查或人工回退的任务资料。任务不会在后台自动删除。
干净恢复保留的另一份 overlay 不计入普通任务文件占用，也不随历史清理删除；
对应任务受到保护，需要单独确认「删除暂存 overlay」后才能删除任务。

普通迁移流程为：

1. 校验压缩包 CRC、布局、路径、重复条目、展开大小和剩余空间，生成计划。
2. 用户确认后校验文件摘要，把待恢复文件写入持久存储。
3. 如果选择了缺失的 SmartDNS/Nikki，先从当前 myfeed 准备它们，避免备份 DNS/代理配置在重启后缺少运行程序。
4. 逐个保存原始文件、原子替换目标文件、记录进度；发生写入错误时尝试恢复原始文件。
5. 等待实际重启，在新一次启动中安装软件、修复主题和服务。

检查阶段不更改当前配置文件；原始压缩包和临时展开文件位于 `/tmp`。
检查后尚未确认的任务如果遇到重启，需要重新上传或选择路由器上的备份检查。
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
干净环境的引导软件会保留 `@myfeed` 约束，因此默认源与同地址的标签定义同时保留，
让普通 APK 安装和升级也能解析这些约束。旧干净恢复任务在软件重试时会补齐缺失的标签定义。
不会使用 `--force-broken-world`。必需软件失败会保留失败状态；可选软件失败单独显示警告。
移除列表只接受 LuCI 应用、翻译、协议页面和主题，并禁止移除恢复工具及基础界面。
选中的应用、翻译和主题在同一次 APK 事务中移除，避免语言包或主题配置依赖阻止删除。
执行后逐包检查实际安装状态；仍被未选中软件依赖的包会保留并报告失败。
软件阶段结束前再次核对删除结果和失败的安装项；后续依赖安装已补齐的软件会更新为成功，
被后续依赖重新安装的待移除软件仍会报告失败，查询超时也不会算作已删除。

## 恢复前重建干净 overlay

此选项默认关闭。开启后，软件替代手动清理当前 overlay、重新挂载并运行恢复脚本的步骤，
使用当前固件的基础系统和 APK 数据库重新建立环境。支持当前内部或外部 ext4/f2fs overlay，
也支持升级后尚未启用 extroot 的外部 ext4/f2fs 分区。当前根文件系统须采用
squashfs + OverlayFS 的标准 upper/work 布局；平铺 ext4 根文件系统和自定义布局不自动处理。

使用时保持「保留当前 extroot」和「迁移完成后自动重启」开启，选择备份并重新检查计划。
在「恢复目标 overlay」中选择当前 overlay，或按设备名、文件系统、标签和 UUID 选择外部分区。
也可点击「打开 DiskMan 选择分区」，在嵌入的磁盘页面直接点击分区条，或点击「查看分区」后点击分区行；
选择后自动关闭弹窗并回填设备，下方显示文件系统及 UUID。DiskMan 入口需已安装 `luci-app-diskman`
且其独立页面可访问；不可用时仍可使用下拉框。弹窗用于分区选择，挂载和系统切换在确认恢复后执行。
分区可以尚未挂载，也可以仅挂载为数据目录。分区列表不包含当前系统、ROM 和启动分区。
默认选择「当前 overlay」，无需再选择外部分区即可保存设置或检查备份。
启用新 extroot 时，确认页面显示目标设备和 UUID，软件自动按 UUID 更新挂载配置；
无需手动格式化、重新挂载或运行脚本。
仅保存设置或检查备份不会清理系统；确认「执行恢复」后按以下流程执行：

开启重建功能后，待恢复文件直接暂存到所选 overlay 分区，当前系统只保留任务记录。
升级后尚未启用外部 overlay 时，空间检查和文件暂存也使用所选外部分区，
不会要求内部 overlay 同时容纳整份恢复文件。空间不足的报错显示检查位置、可用容量和所需容量。

1. 在选定文件系统的独立暂存目录准备新 upper/work；尚未挂载的外部分区临时挂载，原系统继续运行。
2. 从当前固件的 APK 数据库及软件源安装恢复工具和所需 DNS/代理程序，再按计划写入备份配置及自定义文件。
3. 通过 procd 停止服务、进入 RAM、卸载当前 overlay，再交换新旧 upper/work；需要时启用选定分区为 extroot，然后重启。此过程不刷写固件、不格式化磁盘。
4. 新环境启动后继续安装计划中的软件、移除指定 LuCI 软件并修复服务；页面重新连接后可查看结果和日志。

软件源不可访问、磁盘空间不足或布局不支持时，准备阶段失败并保留当前环境。
需要有空间存放新环境，旧环境不会立即释放；`upper/work` 之外的外部磁盘目录保留。
保留目录位于 `/overlay/.overlay-restore-clean/TASK_ID/`，同一时间只保留一组切换环境。
完成后可选择「回退到清理前环境」并重启，或验证完成后确认「删除暂存 overlay」释放空间。
回退后，恢复出的环境仍保留在暂存目录；删除暂存目录会永久删除其中的文件。
新启用外部分区的任务回退到原系统，并恢复原 fstab 和外部分区旧环境；
升级遗留的 extroot UUID 标记会备份并在切换时更新，回退时恢复。
若此后升级了固件，自动回退会停用，仍可在设备布局匹配时删除暂存环境。

命令行对应入口：

```sh
uci set overlay_restore.main.clean_overlay=1
uci set overlay_restore.main.keep_current_extroot=1
uci set overlay_restore.main.reboot=1
# 默认使用当前 overlay。
uci set overlay_restore.main.overlay_device=''
# 升级后需要重新启用外部分区时，先列出分区，再将上一行改为所选设备。
overlay-restore devices
# 例如：uci set overlay_restore.main.overlay_device='/dev/sda1'
uci commit overlay_restore
overlay-restore inspect /mnt/backup/overlay_backup.tar.gz
overlay-restore apply TASK_ID --confirm TASK_ID
overlay-restore rollback TASK_ID --confirm TASK_ID
overlay-restore discard-overlay TASK_ID --confirm TASK_ID
```

## 配置与服务

配置文件为 `/etc/config/overlay_restore`，主 section 为 `main`、类型为 `restore`。
网页可编辑软件安装/移除列表、备份大小限制、extroot/网络/凭据选项和 IPTV 参数。
「IPTV 数据目录」与「Home Assistant 配置目录」均使用 QuickFile 选择，也可手填绝对路径。
点击目录旁的「打开 QuickFile 选择目录」，在嵌入页面中打开目标目录，
再点击「使用当前目录」即可回填。QuickFile 使用现有 LuCI 登录会话，按其现有文件管理权限运行。
现有 `00-myfeed.list` 优先于默认源，源地址会在检查时冻结。
默认上传上限 256 MiB、展开上限 2048 MiB，实际可处理大小还受 `/tmp` 和 overlay 空间限制。
普通迁移在当前系统为待写入文件、原始文件和原子替换预留空间；重建干净 overlay 时，
在所选分区为待恢复文件和新环境预留空间，并保留旧环境用于回退。

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

RPC 暴露 `prepare/list/status/apply/retry/devices/usage/cleanup/remove/rollback/discard_overlay`；`prepare` 可接收备份绝对路径，
省略路径时仍使用固定上传临时文件名。直接选择的备份不会被删除，文件符号链接及特殊文件会被拒绝。
ACL 的只读部分允许读取任务与配置、查询所选路径的文件属性。
写入部分允许恢复和修改本应用配置，上传只允许固定临时文件名；
不授予任意命令、任意文件内容读写、删除或其它 UCI 配置的权限。

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
