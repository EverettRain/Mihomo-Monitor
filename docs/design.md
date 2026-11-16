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

1. **进程扫描**（`sysinfo` crate）：匹配进程名 `mihomo*`、`verge-mihomo*`、`clash-meta*`、`FlClashCore` 等；能读到命令行时解析 `-d`、`-f`、`-ext-ctl`、`-ext-ctl-unix`、`-ext-ctl-pipe`、`-secret`，再读配置文件里的 `external-controller*` 与 `secret`。
   - 限制：Clash Verge Rev 的“服务模式”下内核以 SYSTEM / root 运行，普通用户读不到它的命令行，所以还需要第 2 步。
2. **已知客户端适配器**：每个客户端一条规则，给出它固定的控制地址和配置文件位置。

   | 客户端 | 控制接口 | 状态 |
   | --- | --- | --- |
   | Clash Verge Rev | 见下方说明；若开启了 HTTP 控制器（默认 `127.0.0.1:9097`）则读 `clash-verge.yaml` | 已对照源码，旧版路径【待实测】 |
   | Mihomo Party / Sparkle | 自有 pipe / socket | 【待实测】 |
   | FlClash | 内核独立进程，默认不开放 external controller，需要用户在设置里打开 | 【待实测】 |
   | ClashX Meta | `~/.config/clash.meta/config.yaml` 中的 HTTP 地址 | 【待实测】 |

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
  - 进程：`metadata.process`。只有内核开启了进程查找（TUN 模式下通常默认开启，或 `find-process-mode: always`）才有值，否则记为“未知进程”，界面提示如何开启。
  - 入站：`metadata.inboundName`，如用户为容器单独开的监听端口（参考工具里的 `US-Workstation-Port`）。进程为空的连接（容器、局域网设备经 OrbStack / allow-lan 转发）靠它区分来源，取值很少，成本可忽略。
  - 域名：`metadata.host`，为空时用 `destinationIP`。按精确主机名入库，不在入库前合并子域名；界面提供“按主域名合并”开关（eTLD+1，`psl` crate），只对保留下来的数据合并。
- **省资源的读法**：
  - 后台只开一条 `/connections` WebSocket（默认 2 秒间隔），它同时给出总量与拆分；只用 serde 反序列化需要的字段。
  - `/traffic` 的秒级速率只在窗口打开、或 macOS 菜单栏开了实时速率时订阅。
  - 增量先在内存累加，每 60 秒（以及退出时）合并写入一次 SQLite。

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
- 订阅以名称标识（provider 名，或客户端里的配置名）；用户改了订阅名，可在周期设置里把旧名称的历史合并过来。

周期计算是纯函数（输入：规则、时区、当前时刻；输出：周期起止），重点测试月末、闰年和夏令时。

## 8. 代码结构

沿用 Hydra 的工程骨架（pnpm + Vite + React + Tailwind + lucide，GitHub Actions 发布与 updater），Rust 侧按层分目录：

```text
app/
  src/                      React：pages/、components/（图表沿用 Hydra 的手写 SVG）、lib/api.ts
  src-tauri/src/
    domain/                 纯逻辑：增量与重启判断、15 分钟分桶、周期计算、按时区归并
    app/                    用例：采集服务、发现流程、各视图查询、配额提醒
    infra/
      mihomo/               API 客户端：TCP / unix socket / named pipe 上的 HTTP 与 WebSocket
      discovery/            进程扫描 + 各客户端适配器
      store.rs              SQLite
    shell/                  Tauri：命令、托盘、窗口生命周期
```

## 9. 里程碑

每一步都能端到端使用：

1. **骨架**：托盘常驻、懒创建窗口、单实例、开机自启、SQLite 初始化。
2. **总量采集**：Clash Verge Rev 适配器 + 手动添加；`/traffic` 累计、检查点、15 分钟桶；托盘显示今日用量。
3. **vnstat 视图**：日 / 周 / 月 / 年 / 小时、实时曲线、热力图。
4. **按维度拆分**：`/connections` 采集；节点 / 订阅 / DIRECT / 进程排行；域名按天前 N 名；“未归类”。
5. **计费周期**：每订阅一个周期、配额提醒、机场 `subscriptionInfo` 对照。
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
2. Mihomo Party / Sparkle、FlClash、ClashX Meta 的控制地址与 secret 位置。
3. ~~`chains` 顺序~~ 已确认；`providerChains` 在使用 `proxy-providers` 的配置下的取值还没见过。
4. 1000 条以上连接时，2 秒间隔读 `/connections` 的 CPU 占用，据此定默认间隔。参考工具每 2 秒读一次，C 实现 RSS 3.2 MB、CPU 0–0.1%，说明这个频率本身不贵；Tauri 版常驻进程预计 20–30 MB，主要是 Tauri 运行时和托盘本身的开销。
5. ~~服务模式下 Verge 的 `profiles.yaml` 位置与字段~~ 已确认（第 7 节）。
