<div align="center">

<img src="app/src-tauri/icons/icon.png" alt="Mihomo-Monitor" width="120">

# Mihomo-Monitor

**像 vnstat 一样统计 mihomo 内核的流量：直接读取你已经在用的 Clash 客户端，常驻托盘，几乎不占资源。**

基于 Tauri 2（Rust + React）构建，支持 Windows 与 macOS。

![Windows 10 | 11](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D4?style=flat-square&logo=windows)
![macOS](https://img.shields.io/badge/macOS-11%2B-000000?style=flat-square&logo=apple)
![Tauri 2](https://img.shields.io/badge/Tauri-2.x-24C8DB?style=flat-square&logo=tauri)
![License: MIT](https://img.shields.io/badge/License-MIT-2e7d4a?style=flat-square)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/overview-dark.png">
  <img src="docs/images/overview-light.png" alt="Mihomo-Monitor 概览页" width="760">
</picture>

</div>

## 目录

- [功能概览](#功能概览)
- [快速开始](#快速开始)
- [内核发现](#内核发现)
- [历史](#历史)
- [分布](#分布)
- [域名分组](#域名分组)
- [订阅与计费周期](#订阅与计费周期)
- [统计口径](#统计口径)
- [后台常驻](#后台常驻)
- [数据存储](#数据存储)
- [开发](#开发)
- [发布](#发布)
- [路线图](#路线图)
- [已知限制](#已知限制)

## 功能概览

| 功能 | 说明 |
| --- | --- |
| 直接读取已有内核 | 自动找到 Clash Verge Rev 等客户端正在运行的 mihomo 内核，通过它的控制接口读取流量计数；不内置内核，不接管、不改动你的代理 |
| vnstat 式记账 | 按 15 分钟记录、永久保存，按本地时间汇总成小时、日、周、月、年 |
| 不漏记 | 以内核自身的累计计数（`/connections` 的 `uploadTotal` / `downloadTotal`）为准；Mihomo-Monitor 退出或电脑睡眠期间，只要内核没有重启，流量会在下次读取时补回 |
| 实时速率与热力图 | 最近 5 分钟的下载与上传速率曲线，一年每天流量的热力图 |
| 历史 | 按小时、日、周、月、年翻看任意时段，用量最高的日子排行，自定义起止日期，导出 CSV（对应 vnstat 的 -h / -d / -w / -m / -y / -t） |
| 分布 | 流量去了哪里：按实际出站节点、订阅、进程、入站和域名排行，展开进程可看它访问的域名 |
| 域名分组 | 给域名与 IP 打组（例如 YouTube、公司内网），分布页按分组合并显示；支持粘贴文本、CSV、JSON、Clash / sing-box 规则集导入 |
| 计费周期 | 每个订阅设置重置日与配额，显示本周期用量、预计周期末用量，用到 80% / 100% 时发系统通知，并与机场返回的已用量对照 |
| 多个内核 | 本机与路由器（OpenClash 等）的内核可以同时统计，分别查看或合计 |
| 常驻托盘 | 开机自启，托盘悬停显示今日用量；轻量模式下关闭窗口会连界面一起释放 |

## 快速开始

1. 安装并打开 Mihomo-Monitor。首次启动会开启开机自启，之后它在托盘（菜单栏）里运行。
2. 打开你平时用的 Clash 客户端（例如 Clash Verge Rev）。几秒内 **内核** 页会出现这个内核，状态为「正在统计」。
3. **概览** 页的「总览」显示今天、本周、本月、今年、近 30 天、日均、峰值单日与累计用量，最近 5 分钟的实时速率，以及一年的流量热力图；「趋势」按今天每小时或近 7 / 30 / 90 天每天显示下载与上传，悬停查看数值，也可以切换成表格。同时统计多个内核时，可以分别查看或合计。

第一次连接某个内核时，它此前累计的流量不会计入（和 vnstat 第一次看到网卡时一样），统计从这一刻开始。

<div align="center">
<img src="docs/images/trend.png" alt="概览页的趋势视图" width="720">
</div>

<div align="center">
<img src="docs/images/sources.png" alt="内核页" width="720">
</div>

## 内核发现

Mihomo-Monitor 依次尝试下面几种方式，找到的每个候选都会先用 `GET /version` 确认是 mihomo 内核：

| 方式 | 说明 |
| --- | --- |
| 读取内核进程的启动参数 | 客户端通常用 `-d`、`-f`、`-ext-ctl-unix`、`-ext-ctl-pipe` 启动内核；Mihomo-Monitor 由此找到控制接口，必要时再读配置文件里的 `external-controller` 与 `secret` |
| Clash Verge Rev 的已知位置 | 读不到启动参数时（例如 Windows 上以 SYSTEM 运行的服务模式），macOS 检查用户临时目录与服务目录下的 `verge-mihomo*.sock`，Windows 列出以 `verge-mihomo` 开头的命名管道 |
| 常用端口 | 以上都没有结果时，探测 `127.0.0.1` 的 9090 与 9097 端口；设置了 secret 时提示手动添加 |
| 手动添加 | 在 **内核** 页填写控制地址与 secret，适合路由器上的内核或没有被自动发现的内核 |

unix socket 与 Windows 命名管道不需要 secret（mihomo 不在这两个通道上校验 secret，靠文件权限限制访问），所以本机的 Clash Verge Rev 通常不用做任何设置。只想统计路由器时，可以在 **内核** 页关闭「自动发现本机内核」。

已实测：macOS 上的 Clash Verge Rev 2.5.6（服务模式，mihomo v1.19.31）。其他客户端（Mihomo Party、FlClash、ClashX Meta）与 Windows 上的 Clash Verge Rev 尚未逐一实测，欢迎反馈。

## 历史

<div align="center">
<img src="docs/images/history.png" alt="历史页" width="720">
</div>

| 视图 | 一页显示 | 对应 vnstat |
| --- | --- | --- |
| 小时 | 一天的每个小时 | `-h` |
| 日 | 一个月的每一天 | `-d` |
| 周 / 月 | 一年的每周、每月 | `-w` / `-m` |
| 年 | 全部年份 | `-y` |
| 排行 | 用量最高的 20 天 | `-t` |
| 自定义 | 任意起止日期，按天列出 | — |

每个视图有合计、柱状图和明细表（下载、上传、合计、占比、平均速率），可以前后翻页，也可以导出 CSV（带 BOM，Excel 直接打开不乱码）。平均速率按这段时间已经过去的部分计算。

## 分布

<div align="center">
<img src="docs/images/breakdown.png" alt="分布页" width="720">
</div>

| 维度 | 来源 |
| --- | --- |
| 节点 | 连接链路的第一跳（`chains[0]`），即实际出站的节点；直连显示为 DIRECT |
| 订阅 | 节点来自 proxy-provider 时取 provider 名；Clash Verge Rev 会把订阅展开成普通节点，所以取它当时启用的配置名（`profiles.yaml` 的 `current`）；直连不属于任何订阅 |
| 进程 | 发起连接的程序。需要内核能查到进程：TUN 模式下通常默认可以，否则在配置中设置 `find-process-mode: always` |
| 入站 | 流量从内核的哪个入口进来，例如混合端口、TUN，或给容器、局域网设备单独开的端口 |
| 域名 | 连接的域名，没有域名时是 IP；可以按主域名合并（`a.cdn.example.co.uk` 计入 `example.co.uk`） |

时间范围可选今天、昨天、本周、本月、近 30 天、今年。域名可以按分组（见下一节）、按主域名或原样显示；展开分组可看其中的域名，域名行末的按钮可以直接把它加入分组。

## 域名分组

<div align="center">
<img src="docs/images/groups.png" alt="域名分组页" width="720">
</div>

一堆 IP 和陌生域名很难看出属于哪个服务。**域名分组** 页可以把它们归到一个名字下，分布页按分组显示时，同一分组的流量合并成一行。页面列出所有分组，点「编辑」进入某个分组的编辑页，在那里添加、修改、删除条目，导入到这个分组，或者改名、删除分组；按 Esc 返回列表。

条目的写法与 Clash 规则相同：

| 匹配方式 | 写法 | 匹配 |
| --- | --- | --- |
| 域名及子域名 | `youtube.com`、`+.youtube.com`、`DOMAIN-SUFFIX,youtube.com` | 这个域名和它的所有子域名 |
| 仅此域名 | `DOMAIN,www.google.com` | 只有这个域名 |
| 包含关键字 | `DOMAIN-KEYWORD,youtube` | 域名中含有这段文字 |
| IP / 网段 | `149.154.167.51`、`91.108.4.0/22`、`IP-CIDR,…` | 地址落在这段范围内 |

一个域名同时符合多条时，依次按仅此域名、最长的子域名规则、关键字判断；地址按最长的网段判断；同一条目出现在多个分组时，先建的分组优先。

**导入**：可以粘贴文本或选择文件，导入前会预览识别结果，并列出无法识别的行和原因：

| 格式 | 说明 |
| --- | --- |
| 文本 | 每行一条：域名、IP、网段、网址（取其中的域名）、Clash 规则行、`+.example.com`；`#` 开头的是注释 |
| CSV / TSV | 分组、条目、类型三列，有无表头均可（表头可用 group / 分组、domain / 域名、type / 类型 等） |
| JSON | 字符串数组、`{"分组": ["条目", …]}`、对象数组、sing-box rule-set，以及本应用导出的格式 |
| YAML | Clash rule-provider（`payload:`，domain、ipcidr、classical 三种都可以） |

没写分组的条目加入导入时选择的分组；写了分组的按名字归入，不存在的分组自动新建。导入后每一条都可以单独修改匹配方式和内容、删除，也可以继续追加。**导出** 会把全部分组保存为 JSON，再导入即可还原。

## 订阅与计费周期

<div align="center">
<img src="docs/images/subscriptions.png" alt="订阅页" width="720">
</div>

**订阅** 页列出有流量经过的订阅（订阅的认定方式见上一节），可以为每个订阅设置计费周期：

| 设置 | 说明 |
| --- | --- |
| 重置 | 每月第 N 日的某个时刻（N 大于当月天数时在月末重置），或从某天起每隔 N 天；时刻以 15 分钟为单位。新建时默认取机场到期日的日期，多数机场按购买日每月重置 |
| 计量 | 上传 + 下载（多数机场如此），或仅下载 |
| 配额 | 可选，默认取机场返回的总量 |
| 提醒 | 用到配额的 50% / 80% / 90% / 100% 时发系统通知，默认 80% 与 100%；每个周期每条提醒线只通知一次 |

设置后显示本周期的起止、已用、剩余、按目前速度预计的周期末用量和日均；概览页与托盘悬停文字也会显示各周期的用量。

机场数据（已用、总量、到期时间）来自 Clash Verge Rev 保存的订阅信息，或内核里 proxy-provider 的订阅信息，与本地统计并排显示。两者通常不同：机场统计的是使用这个订阅的所有设备，还可能按节点倍率计费；开始按连接统计之前的流量也无法区分订阅。

## 统计口径

- **总量取自内核的累计计数。** 每次读数与上一次相减得到增量；计数变小说明内核重启过，此时当前计数全部是新流量。
- **断点续算。** 每分钟把增量和当前计数（检查点）在同一个事务里写入数据库。Mihomo-Monitor 重启后，如果内核还是同一个进程（进程号与启动时间都没变），中间的流量从计数差补回；内核换了进程且启动在检查点之后，它的全部计数都算新流量。内核与 Mihomo-Monitor 同时停止期间的流量无法找回。
- **补回的流量记在补回的时刻。** 中间这段时间里每一刻用了多少已经无从得知，所以按小时看时它会集中在恢复读取的那一格。
- **时区。** 数据按 UTC 存成 15 分钟一格。所有时区与 UTC 都相差整数个 15 分钟，所以换时区后按天汇总依旧准确；夏令时当天按 23 或 25 小时计算。
- **单位。** 1 GB = 1024 MB，与多数代理客户端和机场的计法一致。
- **按连接拆分。** 每 2 秒读取一次 `/connections`：总量与各连接的计数取自同一时刻，每条连接的增长记到它的节点、订阅、进程、入站和域名上。连接在两次读取之间就关闭时，最后一段看不到去向，记为「未归类」；节点维度加起来始终等于总量，其他维度不足的部分也显示为「未归类」（进程维度显示为「未知进程」）。开始按连接统计之前的历史都是「未归类」。
- **域名的保留。** 今天的域名明细完整；过去的每一天只保留用量最高的 200 个（进程 × 域名），每个进程至少保留它自己的前 10 个，其余并入该进程的「其他」，进程的总量不受影响。
- **多个内核合计。** 同一份流量如果先后经过两个内核（例如本机内核的出站又经过路由器的内核），合计时会重复计算，概览页会提示。

## 后台常驻

- 主窗口在需要时才创建。**轻量模式**（默认开启）下关闭窗口会销毁界面，只留下 Rust 后台进程继续统计；关闭轻量模式时关窗只是隐藏，再打开更快。
- macOS 上没有窗口时，应用不出现在程序坞与切换器里。
- 后台对每个内核保持一条 `/traffic` 连接（每秒一行，用于实时速率），并每 2 秒读取一次 `/connections`（用于记账与拆分）。增量在内存中累加，每分钟写一次盘；界面关闭时不向前端推送任何数据。
- 开机自启以 `--tray` 参数启动，不弹出窗口。

## 数据存储

| 系统 | 位置 |
| --- | --- |
| macOS | `~/Library/Application Support/Mihomo-Monitor` |
| Windows | `%LOCALAPPDATA%\Mihomo-Monitor` |

目录中有 `traffic.db`（SQLite，WAL 模式）、`settings.json` 与 `logs/`。可以用环境变量 `MIHOMO_MONITOR_HOME` 改用其他目录。数据库结构用 `PRAGMA user_version` 标记版本，升级时自动迁移。

## 开发

需要 Node.js 22、pnpm 11 与 Rust 1.95（`app/rust-toolchain.toml` 会自动选用）。

```bash
cd app
pnpm install
pnpm tauri dev          # 开发运行
pnpm typecheck          # 前端类型检查
pnpm test:rust          # Rust 测试
```

没有装 Clash 客户端也可以试用：用模拟内核，然后在 **内核** 页手动添加 `127.0.0.1:19090`。

```bash
python3 app/scripts/fake-mihomo.py --port 19090
```

代码按层组织（`domain` → `infra` → `service` → `shell`），设计说明见 [docs/design.md](docs/design.md)。

### README 截图

```bash
app/scripts/readme-screenshots.sh
```

只能在 macOS 上运行，使用已安装的 `/Applications/Mihomo-Monitor.app`。脚本用 `MIHOMO_MONITOR_HOME` 指向示例目录、关闭自动发现、只连模拟内核，并用 `app/scripts/demo-data.py` 生成一年的虚构流量，不会读到或改动你自己的内核与统计。需要给终端授予「辅助功能」与「屏幕录制」权限。

## 发布

1. 在仓库的 **Settings → Secrets and variables → Actions** 中添加 `TAURI_SIGNING_PRIVATE_KEY`（updater 签名私钥的内容，无密码）。对应的公钥已写在 `app/src-tauri/tauri.conf.json`。
2. 修改 `app/package.json`、`app/src-tauri/Cargo.toml` 与 `app/src-tauri/tauri.conf.json` 中的版本号并提交。
3. 推送 `vX.Y.Z` 标签，`release.yml` 会构建 Windows 与 macOS（Apple Silicon、Intel）安装包，生成更新说明并发布；应用内的更新检查读取最新 Release 的 `latest.json`。

想在发布前试装某个提交，可以在 Actions 页手动运行 **Test build**。

## 路线图

- [x] 托盘常驻、轻量模式、开机自启、自动更新
- [x] 内核发现（启动参数、配置文件、Clash Verge Rev 已知位置、手动添加）
- [x] 总量统计：日 / 周 / 月 / 年、实时速率、断点续算
- [x] 历史浏览：小时 / 日 / 周 / 月 / 年、排行、自定义时段、导出 CSV
- [x] 按节点、订阅、DIRECT、进程、入站、域名拆分
- [x] 按订阅设置计费周期与配额，提醒，对照机场返回的已用量
- [x] 域名分组：多种格式导入、逐条编辑、导出，分布页按分组显示
- [ ] 订阅改名后合并旧名称的历史
- [ ] 更多客户端的实测与适配

## 已知限制

- 第一次连接某个内核时，它此前的累计流量不计入。
- 远程内核（手动添加的地址）看不到进程，只能靠计数是否变小来判断重启：如果它在 Mihomo-Monitor 停止期间重启，并且新的计数已经超过上次记录的值，这段时间会少记。
- 原版 Clash 与 Clash Premium 不支持，只支持 mihomo（Clash.Meta）内核。
- Clash Verge Rev 的订阅按当时启用的配置记账：全局 Merge / Script 增强从别处加入的节点，也会记到当前配置名下。
- 一个订阅同时在多台设备上使用时，这里只统计本机（或所添加内核）经过的流量，与机场显示的已用量不同。

## License

[MIT](LICENSE)
