# Mihomo-Monitor 设计方案（v1）

> 状态：方向已确认（见第 10 节）。标 **【待实测】** 的地方要在真实客户端上验证（见第 11 节）。

## 1. 目标与约束

| 项 | 要求 |
| --- | --- |
| 做什么 | 统计经过 mihomo 内核的流量，按 年 / 月 / 周 / 日 / 小时 汇总（类似 vnstat）；按节点、订阅、进程、域名拆分；每个机场订阅单独设计费周期与配额 |
| 怎么采集 | 优先发现用户电脑上**已在运行**的 mihomo 内核（Clash Verge Rev、Mihomo Party、FlClash 等），通过它的 external controller API 读数，不接管、不改动用户的代理；**不内置内核**，找不到时引导用户启动客户端或手动填写控制地址 |
| 平台 | Windows 10/11、macOS 11+ |
| 常驻 | 托盘常驻、开机自启；关窗后只剩 Rust 后台进程 |
| 资源目标 | 窗口关闭时 RSS < 30 MB，平均 CPU < 0.5%，磁盘写入 ≤ 1 次/分钟 |
| 技术栈 | Tauri 2（Rust + React），框架沿用 Claude-Hydra |

关于“不带浏览器内核”：Tauri 用的是系统自带的 WebView（Windows WebView2 / macOS WKWebView），不打包 Chromium。为了让常驻时连系统 WebView 都不占内存，提供 Hydra 同款“轻量模式”开关，**默认开启**：开启时关窗即销毁 WebView、打开再重建（约 1 秒）；关闭时关窗只隐藏，再打开更快但一直占着 WebView 内存。

## 2. 进程模型

**主推：单进程 Tauri 应用 + 懒创建窗口**（与 Hydra 一致）

```text
┌──────────── Mihomo-Monitor 进程（常驻）────────────┐
│ Rust 后台：发现内核 → 采集 → 内存累加 → 每分钟落盘     │
│ 托盘：今日用量 / 本周期用量（macOS 菜单栏可显示实时速率）│
│ 窗口（按需创建，关闭即销毁 WebView）：React 仪表板     │
└──────────────────────┬─────────────────────────────┘
                       │ HTTP / WebSocket over TCP · unix socket · named pipe
            ┌──────────┴──────────┐
            │ 用户已有的 mihomo 内核 │（Clash Verge Rev 等）
            └─────────────────────┘
```

- `tauri.conf.json` 里主窗口 `"create": false`，托盘点击时 `WebviewWindowBuilder::from_config` 创建；轻量模式开启时 `CloseRequested` 不拦截、窗口直接销毁，关闭时拦截并隐藏。
- `RunEvent::ExitRequested { code: None }` 时 `prevent_exit()`，保证没有窗口也继续运行。
- 插件：`single-instance`、`autostart`（官方插件，替代 Hydra 自写的 `sys::set_autostart`）、`notification`（配额提醒）、`updater`。

备选：

- **独立守护进程 + GUI 两个程序**：守护进程做成系统服务，GUI 崩溃不影响采集，也能开机未登录就开始统计。代价是两份二进制、安装服务要管理员权限、多一层 IPC。目前两个平台都是登录后才用代理，收益不大。
- **纯原生 UI（egui / Slint）**：完全不用 WebView，但图表和界面开发成本高很多，也没法复用 Hydra 的前端组件。懒创建窗口已能让常驻时不占 WebView 内存，收益不大。

## 3. 内核发现

目标是“打开即用”，按下面顺序尝试，每找到一个候选就用 `GET /version` 验证（返回 `{"meta":true,"version":…}` 才算 mihomo）。

1. **进程扫描**（`sysinfo` crate）：匹配进程名（或可执行文件名）`mihomo`、`mihomo-alpha`、`mihomo-smart`、`verge-mihomo*`、`clash-meta`、`FlClashCore`、`com.metacubex.ClashX.ProxyConfigHelper.meta` 与发布版的 `mihomo-<os>-<arch>-…`；能读到命令行时解析 `-d`、`-f`、`-ext-ctl`、`-ext-ctl-unix`、`-ext-ctl-pipe`、`-secret`，再读配置文件里的 `external-controller*` 与 `secret`。
   - 限制：Clash Verge Rev 的“服务模式”、ClashX Meta 的助手都以 SYSTEM / root 启动内核，普通用户可能读不到它的命令行，所以还需要第 2 步。
   - 客户端由进程名、可执行文件路径、`-d` / `-f` 目录与 socket / pipe 路径识别（`infra::discovery::Client`）。认出客户端后，来源的 key 固定为客户端名（如 `clash-party`），控制接口换位置也不影响历史；Clash Party、Sparkle 用的是普通的 `mihomo`，只能靠路径认出。
2. **已知客户端适配器**：每个客户端一条规则，给出它固定的控制地址。命令行读不到，或某个客户端的内核进程根本看不到时，都按这些位置逐个用 `/version` 试。以下依据各项目 2026-11 的源码：

   | 客户端 | 内核与启动方式 | 控制接口 | 状态 |
   | --- | --- | --- | --- |
   | Clash Verge Rev | `verge-mihomo*`，`-ext-ctl-unix` / `-ext-ctl-pipe` | 见下方说明 | macOS 服务模式已实测 |
   | Clash Party（`mihomo-party-org/clash-party`） | `resources/sidecar/` 下的 `mihomo` / `mihomo-alpha` / `mihomo-smart`，`-ext-ctl-unix` / `-ext-ctl-pipe` | `/tmp/mihomo-party-<uid>-<pid>.sock`（旧版 `/tmp/mihomo-party-<uid>.sock`、`/tmp/mihomo-party.sock`、`-admin.sock`）；`\\.\pipe\MihomoParty\mihomo-{admin,user}-<会话>-<pid>` | 源码 |
   | Sparkle（`xishang0128/sparkle`） | `resources/sidecar/` 下的 `mihomo` / `mihomo-alpha`，或用户指定的系统内核 | `/tmp/sparkle-mihomo-api.sock`、`-api-noperm.sock`、`/tmp/sparkle-mihomo-external.sock`（系统内核）；`\\.\pipe\Sparkle\mihomo` | 源码 |
   | ClashX Meta（`meta` 分支） | `com.metacubex.ClashX.ProxyConfigHelper.meta`，由 launchd 以 root 启动，`-d ~/.config/clash.meta -f <配置>` | 配置未写 `external-controller-unix` 时用随机的 `/tmp/clashx_meta_<5 位>.sock`；另有配置里的 HTTP（默认 `127.0.0.1:9090`） | 源码 |
   | Clash Nyanpasu（经 `nyanpasu-service`） | `mihomo -m -d <运行目录> -f <运行目录>/config-<n>.yaml` | 写进该配置的 `external-controller-unix` / `-pipe`（`core-<n>.sock`），从配置文件读出 | 源码 |
   | FlClash | `FlClashCore` | 主动清空 `external-controller-unix` / `-pipe`；HTTP `127.0.0.1:9090` 只在用户打开「外部控制器」且设置了密钥时开放。读不到密钥，发现后提示用户手动添加 | 源码 |

   **Clash Verge Rev 的控制地址随版本变化，不能写死**。从当前源码（`clash-verge-rev` dev 分支 2026-11-16、`clash-verge-service-ipc` v2.8.1）看，旧版的 `/tmp/verge/verge-mihomo.sock` 已经不用了，而且路径按用户区分：

   | 运行方式 | macOS | Windows |
   | --- | --- | --- |
   | 普通模式（内核作为子进程，与用户同身份） | `$TMPDIR/verge-mihomo.sock`（每个用户自己的临时目录，形如 `/var/folders/…/T/`） | `\\.\pipe\verge-mihomo-sidecar-release-<sha256(用户 SID)>` |
   | 服务模式（内核以 root / SYSTEM 运行） | `/var/run/clash-verge-service/users/<uid>/verge-mihomo.sock`（Verge 2.5.6 实测） | `\\.\pipe\verge-mihomo-<渠道>-<sha256(用户 SID)>` |

   Verge 2.5.6 实测：macOS 服务模式下内核以 root 运行，但普通用户的 `ps` 仍能看到完整命令行，`-ext-ctl-unix` 参数可直接读出。socket 及其所在的 `users/<uid>` 目录归该用户所有（权限 0600 / 0700），所以 Monitor 以同一用户运行就能连上，不需要 secret，其他用户则连不上；`/traffic` 已能读到 `upTotal` / `downTotal`。

   所以对 Verge 的发现顺序是：
   - 普通模式下内核与 Monitor 同一用户，进程命令行可读，直接取 `-ext-ctl-unix` / `-ext-ctl-pipe` 参数，最可靠。
   - 读不到命令行（服务模式）时：Windows 枚举 `\\.\pipe\` 下以 `verge-mihomo` 开头的管道；macOS 检查 `$TMPDIR` 与服务运行目录下的 `verge-mihomo*.sock`。逐个用 `/version` 验证。
   - Windows 的管道带了只允许本用户访问的 SDDL，Monitor 以同一用户运行即可连接。

3. **常用端口探测**：`127.0.0.1` 的 9090 / 9097；返回 401 时提示用户填 secret。
4. **手动添加**：HTTP 地址 + secret，也可填路由器上的 OpenClash / nikki，顺带支持远程内核。

已从 mihomo 源码（`hub/route/server.go`）确认：unix socket（权限 0666）与 Windows named pipe 都**不校验 secret**，所以走这两条路时不需要用户提供 secret。

每个内核实例是一个“数据源”（相当于 vnstat 的网卡），可以同时监控多个，界面可单看或合计。

## 4. 采集

### 4.1 用到的 mihomo API（已对照源码确认字段）

| 接口 | 用途 | 关键字段 |
| --- | --- | --- |
| `GET /traffic`（流式，1 秒 1 条） | 实时速率；仅统计总量时也用它做累计 | `up` `down`（速率）、`upTotal` `downTotal`（内核启动以来累计） |
| `GET /connections?interval=N`（WebSocket） | 按节点 / 订阅 / 进程拆分 | 顶层 `uploadTotal` `downloadTotal`；每条连接 `id` `upload` `download` `chains` `providerChains` `rule` `metadata.process` `metadata.host` |
| `GET /providers/proxies` | 机场侧已用 / 总量 / 到期时间，与本地统计对照 | `subscriptionInfo`：`Upload` `Download` `Total` `Expire` |

### 4.2 增量算法

- **总量以内核累计值为准**：每次读数 `Δ = 当前累计 − 上次累计`；当前值比上次小，说明内核重启过，`Δ = 当前累计`。
- **断点续算**：每次落盘时在同一事务里写入检查点（数据源、内核实例指纹、上次累计）。Monitor 自己重启、崩溃或电脑睡眠后，只要内核没重启，中间的流量能从累计值补回来，不重不漏。内核和 Monitor 同时停掉期间的流量无法找回（vnstat 同样如此）。
- **维度拆分（尽力而为）**：对每条连接记住上次的 `upload/download`，差值记到它的各个维度上。连接在两次快照之间关闭时，最后一小段字节看不到；这部分用“总量 Δ − 各连接 Δ 之和”算出，记为“未归类”，保证各维度相加始终等于总量。Monitor 启动时已存在的连接，以第一次看到的值为基线，之前的字节同样计入“未归类”。
  - **实测依据**：`references/mihomo-process-traffic-c` 只按连接差值累计（2 秒间隔）。以 2026-11-14 21:27 内核重启为起点对比：内核累计 上行 447 MB / 下行 5929 MB，该工具记录约 上行 409–423 MB / 下行 4830–5326 MB（区间来自 21:00 那个小时桶含重启前的流量）。也就是下行少记约 10%–19%，短连接在两次快照之间结束的尾部丢失明显，所以总量必须取内核累计值。
- **每条连接的维度**：
  - 节点：`chains[0]`（实际出站；`DIRECT` / `REJECT` 单独成项）。已实测：`chains` 从实际节点排到最外层策略组，如 `['🇺🇸 美国 xtls-reality', 'US-Global', 'US-Workstation']`。
  - 订阅：`providerChains` 非空时取对应节点的 provider 名。**Clash Verge Rev 实测 `providerChains` 全为空**：Verge 把订阅下载后直接展开成配置里的 `proxies`，不经过 proxy-provider。所以 Verge 的“订阅”= 当时激活的配置（`profiles.yaml` 的 `current`），见第 7 节。
  - 进程：`metadata.process`。只有内核开启了进程查找（TUN 模式下通常默认开启，或 `find-process-mode: always`）才有值；为空时先试归属规则（见 7.2），都不命中则存为空键，界面显示“进程未识别”，并提示如何开启进程查找。
  - 入站：`metadata.inboundName`，如用户为容器单独开的监听端口（参考工具里的 `US-Workstation-Port`）。进程为空的连接（容器、局域网设备经 OrbStack / allow-lan 转发）靠它区分来源，取值很少，成本可忽略。
  - 域名：`metadata.host`，为空时用 `destinationIP`。按精确主机名入库，不在入库前合并子域名；界面提供“按主域名合并”开关（eTLD+1，`psl` crate），只对保留下来的数据合并。
- **读法（已实现）**：
  - 每 2 秒用普通 HTTP 读一次 `/connections` 快照（reqwest 不支持 WebSocket；mihomo 对非 WebSocket 请求返回一次快照）。快照里的 `uploadTotal` / `downloadTotal` 与连接列表是同一时刻的，记账与拆分都用它，只用 serde 反序列化需要的字段。
  - `/traffic` 每个内核常开一条（每秒一行，开销可忽略），只用于实时速率曲线，不参与记账。
  - 增量先在内存累加，每 60 秒（以及退出时）合并写入一次 SQLite。
  - 存储：节点维度保存“未归类”行，各节点相加等于总量；订阅、进程、入站存读到的连接（连接缺这个字段时存为空键），查询时用总量减去这些得到余下部分。订阅、入站把空键和余下部分合称“未归类”；进程分开显示：空键是“进程未识别”（有连接、没有进程名、也没命中规则），余下部分是“未采到连接明细”（内核总量中没有被任何读到的连接计入的部分）。两者在存储里本来就是分开的，旧数据不需要迁移，也不做推测回填。

## 5. 存储

SQLite（`rusqlite` + `bundled`，WAL 模式），放在 Tauri 的 app data 目录。

```sql
sources     (id, kind, name, endpoint, created_at)
dims        (id, kind, key)              -- kind: outbound | subscription | process | inbound | domain
                                         -- key 如 "香港 01"、"DIRECT"、"未归类"、"chrome.exe"、"google.com"
traffic     (source_id, bucket, dim_id, up, down,
             PRIMARY KEY (source_id, bucket, dim_id)) WITHOUT ROWID
                                         -- outbound / subscription / process / inbound；bucket = UTC 秒，按 15 分钟对齐
domain_daily(source_id, day, process_dim_id, domain_dim_id, up, down,
             PRIMARY KEY (source_id, day, process_dim_id, domain_dim_id)) WITHOUT ROWID
                                         -- day = 记录时的本地日期；按（进程, 精确主机名）交叉存
checkpoints (source_id, core_fingerprint, up_total, down_total, at)
cycles      (id, subscription, anchor, quota_bytes, count_mode, alerts)
```

- **一种粒度、永久保留**：节点、订阅、进程、入站用 15 分钟桶，不做分级汇总（rollup）。15 分钟能对齐所有时区（含 +5:30、+5:45），换时区后按天统计依旧准确。容量按参考工具的真实数据估算：48 天里出现 241 个进程，每小时平均 29 行（进程 × 路由）；换成 15 分钟桶、加上节点等维度，约每年 60 万行、20–30 MB，可以接受。
- **域名单独按天存，只留前 N 名**：域名取值成千上万（参考工具 48 天出现约 6000 个主机名、8000 个“进程 × 主机名”组合），按 15 分钟存会让数据库膨胀。当天的（进程, 主机名）用量完整保存在内存并照常每分钟落盘；跨过一天后（或启动时发现有旧的完整日），只保留当天用量前 200 个组合，并且每个进程至少保留它自己的前 10 名，其余按进程合并成“其他”。这样当天数据精确、历史有上限，按进程筛选站点时每个进程的“其他”也能对上该进程的总量。
- 总量 = 该桶所有 `outbound` 维度之和（“未归类”也是一个 outbound），不单独存总量行。
- 日 / 周 / 月 / 年 / 自定义周期都在 Rust 里按本地时区（`jiff` crate，处理夏令时）把桶归到时间段，不在 SQL 里算时区。
- 秒级速率只存在内存的环形缓冲区（最近 10 分钟），供实时图使用。

## 6. 视图（对应 vnstat）

| vnstat | 本项目 |
| --- | --- |
| `-l` 实时 | 实时速率曲线（最近 10 分钟） |
| `-5` / `-h` | 今日按 15 分钟 / 小时柱状图 |
| `-d` / `-m` / `-y` | 日 / 月 / 年列表与柱状图；另加 **周**（周一或周日开始，可设置） |
| `-t` top | 用量最高的日期 |
| — | 一年活动热力图（复用 Hydra 仪表板的热力图） |
| — | 计费周期卡片：本周期已用 / 配额、预计周期末用量、机场侧数据对照 |
| — | 任意时间段内按节点 / 订阅 / DIRECT / 进程 / 入站 / 域名的排行与占比；选中某个进程时列出它访问的站点（对应 `mistat sites <进程>`） |

托盘：Windows 悬停显示今日与本周期用量；macOS 菜单栏可选显示实时速率。

## 7. 计费周期

每个机场订阅单独设一个周期，统计范围就是经过该订阅节点的流量。“订阅”按来源分两种：

- **proxy-provider**（配置里写了 `proxy-providers` 的情况）：连接的 `providerChains` 直接给出订阅名；机场用量来自 `/providers/proxies` 的 `subscriptionInfo`。
- **客户端的“配置 / 订阅”**（Clash Verge Rev 的常见用法）：内核里看不到订阅，节点是展开后的。采集时记下客户端当前激活的配置（Verge：`profiles.yaml` 的 `current` 指向的条目，名称取其 `name`），这段时间所有代理流量（非 `DIRECT` / `REJECT`）记到该订阅。机场用量取该条目的 `extra`（`upload` `download` `total` `expire`，Verge 源码 `PrfExtra`）。用户切换配置后，之后的流量自然记到新订阅上。
  - Verge 2.5.6 实测（服务模式）：`profiles.yaml` 仍在 `~/Library/Application Support/io.github.clash-verge-rev.clash-verge-rev/`；`current` 是条目的 `uid`；条目 `type` 有 `remote`（机场订阅，带 `extra`）、`local`（本地配置，无 `extra`），以及 `merge` / `script` / `rules` / `proxies` / `groups` 等附属增强项（`name` 为 null，不算订阅）。
  - `local` 配置也能设周期（例如自建 VPS 的月流量），只是没有机场数据，配额手填。
  - 已知局限：全局 Merge / Script 增强若从别处注入节点，这些节点的流量仍记到当前配置名下。

周期设置：

- **重置规则**：每月第 N 天的 HH:MM（N 大于当月天数时取月末），或从某天起每 N 天。新建时用机场返回的到期时间的日期作为默认重置日（多数机场按购买日每月重置）。参考工具的 `billing 26 17:00` 只支持整点，这里因为是 15 分钟桶，可精确到 15 分钟。
- **计量方式**：上行 + 下行（默认，多数机场如此），或仅下行。
- **配额**：默认取机场返回的总量，可手动改；达到 80% / 100% 时发系统通知（阈值可改）。
- **对照**：卡片上并列显示本地统计的本周期用量与机场返回的已用量（`Upload + Download`）。两者会有出入：机场按节点倍率计费、其他设备也在用同一订阅、Monitor 没在运行的时段，界面上注明这几种原因。
- 订阅以名称标识（provider 名，或客户端里的配置名）。【未实现】用户改了订阅名后，把旧名称的历史合并过来。

周期计算是纯函数（输入：规则、时区、当前时刻；输出：周期起止），重点测试月末、闰年和夏令时。

实现说明（里程碑 5）：

- 周期存在 `cycles` 表，每个订阅一条（v3 迁移加了唯一索引）；重置规则以带 `kind` 的 JSON 存在 `anchor` 列。
- 本周期用量 = 订阅维度在周期内各内核的合计（同一订阅被多个内核使用时合并），按计量方式取上传 + 下载或仅下载。开始按连接统计之前的流量没有订阅信息，不计入。
- 机场数据：Verge 从 `profiles.yaml` 的 `extra` 读（文件变化才重读）；`proxy-providers` 每 10 分钟读一次 `/providers/proxies`。只保存在内存里。
- 提醒：每分钟写盘后检查一次；每个周期每个阈值只通知一次，记录在 `meta` 表（`alerted:<周期 id>:<周期开始>`）。通知用 `tauri-plugin-notification`。
- 展示：「订阅」页（周期进度、预计周期末用量、日均、机场数据对照、设置对话框），概览页的「计费周期」进度条，托盘悬停文字。

## 7.1 域名分组（新增需求）

用户给域名与 IP 打组，分布页的域名可按分组合并显示。

- 条目沿用 Clash 的四种匹配：`DOMAIN`、`DOMAIN-SUFFIX`、`DOMAIN-KEYWORD`、`IP-CIDR`；优先级同 Clash（精确 > 最长后缀 > 关键字；地址按最长前缀）；同一条目在多个分组时先建的分组优先。纯逻辑在 `domain/hostgroup.rs`。
- 文本里不带前缀的域名按“域名及子域名”读，更贴近“把这个服务归一组”的用意；Clash rule-provider 与 sing-box 里则按各自的原意（裸域名为精确匹配）。
- 导入解析在 `infra/import.rs`：文本、CSV / TSV（`csv` crate）、JSON（含 sing-box rule-set 与本应用导出格式）、YAML（Clash rule-provider）；自动识别格式；逐行报告无法识别的原因，不让整个导入失败。导入先预览、确认后在一个事务里写入。
- 存储：v4 迁移新增 `host_groups`、`host_rules`（同一分组内条目唯一）。分组变化后，内存里编译好的匹配器失效重建。
- 分布页“按分组”时，未命中任何分组的域名按主域名显示。

## 7.3 经代理 / 直连（新增需求）

- 路由（`proxy` / `direct`，空为未区分）是每行流量的属性：`traffic` 与 `domain_daily` 各有一列 `route`，节点、订阅、进程、入站和（进程, 主机名）明细都按它细分，所以任何维度都能只看一条路由；余下部分（没有连接计入的）只在 outbound 行里，路由为空，各路由相加等于总量。按连接实际出站（`chains[0]`）的类型判断：`/proxies` 报告的 `Direct`、`Reject`、`RejectDrop`、`Pass`、`Compatible`、`Dns` 为直连，其余为经代理；类型读不到时按内置名称（DIRECT、REJECT*、PASS）。类型随订阅信息每 10 分钟读一次，出现未知出站时最多每分钟重读一次。订阅维度的 DIRECT 也改用同一判断，自定义直连出站不再计入客户端的订阅。
- 存储：v6 先以独立的 route 维度上线（未发布），v7 改为上面的列：重建两张表（主键加上 route），旧行只按自身的键换算——outbound 按内置名称、subscription 按是否 DIRECT，process、inbound 与域名明细不带路由信息，置空；并删除 v6 的 route 维度行。自定义直连出站在旧记录里会算作经代理，README 已注明。
- 查询：总量、按源合计、概览、历史、排行前几天、各维度排行和域名明细都可以带一个 route 过滤；不带时为全部流量。带过滤时，非节点维度的余下部分只可能是没有路由的旧行（没有连接计入的流量本身没有路由，不在任何一条路由的总量里），标为 `unrouted`，界面显示「未区分是否经代理」，不再显示「未采到连接明细」。实时速率来自 `/traffic`，不能区分。
- 界面：概览、历史、分布右上角「全部 / 经代理 / 直连」，记在本机；图表颜色始终只表示下载、上传。另起一行列出这段时间经代理、直连、未区分的数量和占比，避免只看一类时忘了还有分不出的部分。

## 7.2 归属规则（新增需求）

内核认不出进程的流量（容器、局域网设备经专用入站进来）可以按规则命名。

- 规则：显示名称、匹配条件、优先级、启用。首期条件只有“入站名称精确等于”（区分大小写）。纯逻辑在 `domain/attribution.rs`。
- 只作用于读到了连接、但 `metadata.process` 为空的连接；内核给出的进程名一律保留。不按用户填的名称反查连接，也不按 127.0.0.1、服务域名等多个程序共用的特征推断进程。
- 按优先级数字从小到大试，相同时先建的优先；第一条命中的规则名称作为这条连接的进程，同时用于进程排行与 `domain_daily` 的（进程, 主机名）明细。
- 在读取连接、拆分之前应用，所以只影响之后的采样；已记录的数据不改写，规则改名或删除后旧记录保留当时的名称。只替换进程这个键，不改变任何字节数：各维度与总量在分类前后一致（`split` 与 `monitor` 测试覆盖）。
- 存储：v5 迁移新增 `attribution_rules`。规则变化后，内存里缓存的规则失效重建。

## 8. 代码结构

沿用 Hydra 的工程骨架（pnpm + Vite + React + Tailwind + lucide，GitHub Actions 发布与 updater），Rust 侧按层分目录：

```text
app/
  src/                      React：pages/、components/（图表为手写 SVG）、lib/api.ts
  src-tauri/src/
    domain/                 纯逻辑：counter（增量、重启与断点续算判断）、period（15 分钟分桶、按时区算日/周/月/年）
    infra/                  对外适配：store（SQLite）、settings、mihomo（API 客户端，TCP / unix socket / named pipe）、
                            discovery（进程扫描、启动参数与配置解析、已知客户端位置）、releases（GitHub 更新说明）
    service/                用例：discover（把候选内核逐个用 /version 验证）、monitor（采集、内存累加、定时落盘、各视图查询）
    shell/                  Tauri：托盘、窗口生命周期（轻量模式）、命令
```

## 9. 里程碑

每一步都能端到端使用：

1. ✅ **骨架**：托盘常驻、懒创建窗口、单实例、开机自启、SQLite 初始化。
2. ✅ **总量采集**：Clash Verge Rev 适配器 + 手动添加；累计计数、检查点、15 分钟桶；托盘显示今日用量。
3. ✅ **vnstat 视图**：概览（日 / 周 / 月 / 年、实时曲线、热力图）与历史页（小时 / 日 / 周 / 月 / 年翻页、排行、自定义时段、导出 CSV）。
4. ✅ **按维度拆分**：`/connections` 采集；节点 / 订阅 / DIRECT / 进程 / 入站 / 域名排行（分布页）；域名按天前 200 名；“未归类”。
5. ✅ **计费周期**：每订阅一个周期、配额提醒、机场数据（Verge `extra` / `subscriptionInfo`）对照。
6. **更多客户端与发布**：其余客户端适配器、CI、自动更新、README。

## 10. 已确认的决定

- 统计维度：总量 + 节点 + 订阅 + 进程 + 入站 + 域名。
- 域名按精确主机名存，保留“进程 × 域名”交叉统计。
- 计费周期：每个订阅单独设。
- 平台：Windows + macOS。
- 不内置 mihomo，只做监控。
- 不导入 mistat 的历史数据；两者可并行运行对照。

## 11. 开工前要实测的事项

在装有各客户端的 Windows 与 macOS 上逐项确认，结果补进第 3、4 节：

1. ~~macOS + Verge 2.5.6 服务模式~~ 已确认（第 3 节）；还缺 Windows 上 Verge 的 pipe 名与连接权限，以及 macOS 普通模式。
2. Clash Party / Sparkle、FlClash、ClashX Meta、Clash Nyanpasu 的控制地址已对照源码写进第 3 节，还缺真机确认，特别是 Windows 上 `MihomoParty\…`、`Sparkle\…` 这种带子路径的管道能否由列目录列出。
3. ~~`chains` 顺序~~ 已确认；`providerChains` 在使用 `proxy-providers` 的配置下的取值还没见过。
4. 1000 条以上连接时，2 秒间隔读 `/connections` 的 CPU 占用，据此定默认间隔。参考工具每 2 秒读一次，C 实现 RSS 3.2 MB、CPU 0–0.1%，说明这个频率本身不贵；Tauri 版常驻进程预计 20–30 MB，主要是 Tauri 运行时和托盘本身的开销。
5. ~~服务模式下 Verge 的 `profiles.yaml` 位置与字段~~ 已确认（第 7 节）。
