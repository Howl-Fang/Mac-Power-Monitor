# MacPower

一个面向 macOS 的终端功耗查看器。使用 Rust 编写，编译为单个可执行文件，运行时只依赖系统框架，不需要 root 权限。

English UI / English PDF labels; 本 README 使用中文说明。

## 功能

- 整机功耗：优先读取 SMC `PSTR`，并提供电源输入、电池功率等交叉参考值。
- 电池信息：电量、电压、带符号电流、功率、温度、健康度、循环次数、剩余时间、适配器信息。
- 组件功耗：CPU（E/P 集群）、逐核功耗、GPU、ANE、DRAM、显示、其他 SoC 辅助电源轨。
- 历史曲线：内存环形缓冲、降采样、最小值/平均值/最大值、梯形积分能量（Wh）。
- 持久历史：程序长时间运行（TUI 或无 TUI，`--demo`/`--once` 除外）时自动把紧凑记录写入 `/tmp/macpower/history`，保留最近约 48 小时，跨次启动仍可导出。
- 交互：键盘和鼠标，标签页、滚动、暂停、录制、历史窗口切换。
- 导出：一页式选择来源（最近 48h / 录制段）、时间尺度与格式（PDF / TSV），无需逐级菜单。
- 录制：`r` 开始无上限的完整原始 TSV 录制，保存到 `/tmp/macpower/recordings`；停止后该段独立保留，不再追加。
- 兼容性兜底：SMC、IOReport、电池任意一个不可用时，其余功能仍可工作，缺失值显示 `n/a` 而不是假的 0。

## 环境与构建

要求：

- macOS 12 或更高版本。
- Apple Silicon 或 Intel Mac。
- 从源码构建需要 Rust 1.88+（edition 2021）。

```bash
cargo build --release
./target/release/macpower --help
```

产物为 `target/release/macpower`。运行时不需要 Homebrew、Python 或任何第三方动态库；仅链接 macOS 系统框架。产物是当前架构的单一可执行文件（Apple Silicon 上为 arm64）；本项目未打包 universal binary，在 Intel 上请原生构建。

## 快速开始

```bash
# 默认进入 TUI（stdout 是终端时）
./target/release/macpower

# 只采集一次并输出 TSV
./target/release/macpower --once

# 只采集一次并输出 JSON
./target/release/macpower --once --format json

# 无 TUI 运行 10 秒，输出 TSV
./target/release/macpower --headless --duration 10 --format tsv

# 无 TUI 运行 30 秒，同时保存原始 TSV 和 PDF 报告
./target/release/macpower --headless --duration 30 \
  --raw /tmp/power.tsv --report /tmp/power-report.pdf

# 列出检测到的 SMC key / IOReport channel，便于排查兼容性
./target/release/macpower --list-sensors

# 不访问硬件，用合成数据测试 UI / PDF
./target/release/macpower --demo
./target/release/macpower --demo --once --format json
```

如果 stdout 不是终端且没有指定 `--headless` / `--duration`，程序会自动退化为“采集一次并输出”，适合脚本管道。

## 命令行参数

```text
macpower [OPTIONS]

-i, --interval <MS>      采样间隔，默认 1000；TUI 内可用 +/- 调整
-H, --history <SEC>      内存历史长度，默认 7200 秒（只影响界面实时曲线；/tmp 滚动历史固定保留约 48 小时）
    --raw <PATH>         启动时开始录制完整原始 TSV 到指定路径（不加时不自动录制）
    --report <PATH>      退出时生成 PDF；无 TUI 模式也会生成
    --duration <SEC>     无 TUI 运行指定秒数后退出
    --once               采集一次后退出
    --format <FMT>       tsv | json，配合 --once / --headless
    --headless           强制无 TUI 模式
    --list-sensors       打印传感器探测报告
    --no-mouse           不捕获鼠标，保留终端文本选择
    --no-color           单色 UI
    --ascii              使用 ASCII 图，不依赖盲文/Unicode 字符
    --disable <WHAT>     smc | ioreport | battery，可重复，用于测试兜底
    --demo               合成数据，不访问硬件
-h, --help
-V, --version
```

## TUI 使用

界面包含五个标签页：

- **Overview**：整机功耗与来源、SMC/DC-IN/电池对照、电池仪表、组件功耗条、逐核即时功耗。内容区高度达到 25 行时，额外显示带纵坐标的 60 秒功耗折线图、电池百分比历史，以及每个组件旁的一行 sparkline；空间不足时自动回到紧凑布局。
- **CPU**：CPU 总功耗、E/P 集群、逐核功耗条与最近 60 秒 sparkline。当所有核心都能与图表同时放下时，增加 CPU/E/P 功耗历史图；多核芯片在空间不足时优先保留全部核心行。
- **Power**：全部 IOReport/SMC 电源轨表格，含功率、原始单位、来源和分组。
- **History**：多序列功耗曲线（System / CPU / GPU / Display / DRAM / DC in）、电池电量曲线、`1m / 5m / 15m / 1h / 6h / 24h / 48h / All` 窗口、min/avg/max 与能量统计。短窗口使用内存（`-H`，默认 2 小时）；6h/24h/48h/All 自动从 `/tmp` 滚动历史加载并缓存。
- **Help**：数据来源、公式、按键、鼠标、导出格式、兼容性与警告的完整说明。

功耗进度条采用 `LABEL[||||  数值]` 样式：竖线表示已用部分，空白表示未用部分；数值文字右对齐压在条上，落在填充区内的字符使用该条目的颜色，落在空白区的字符使用普通文字色，因此在窄终端里也能同时看清比例和数值。`--no-color` 下改用加粗区分填充区内的文字。

### 键盘

| 按键 | 作用 |
| --- | --- |
| `q` / `Ctrl-C` | 退出 |
| `?` | 打开/关闭 Help |
| `Esc` | 关闭弹窗；在 Help 中返回上一标签页 |
| `Tab` / `Shift-Tab` | 下一个 / 上一个标签页 |
| `1`–`5` | 直接切换标签页 |
| `h` / `l` | 上一个 / 下一个标签页 |
| `j` / `k` / 方向键 | 滚动 |
| `PageUp` / `PageDown` | 快速滚动 |
| `g` / `G` | 顶部 / 底部 |
| `p` | 暂停 / 恢复采样（恢复时重置 IOReport 差分基线） |
| `r` | 开始 / 停止完整原始 TSV 录制（默认写入 `/tmp/macpower/recordings`） |
| `e` | 打开导出页，选择来源、时间尺度与格式 |
| `+` / `-` | 调整采样间隔（250–10000 ms） |
| `t` | 循环历史窗口：1m → 5m → 15m → 1h → 6h → 24h → 48h → All |
| `m` | 开关鼠标捕获 |

导出页内：`↑`/`↓` 选择时间尺度，`←`/`→` 选择格式，`Tab`/`Shift-Tab` 切换来源，`Enter` 执行导出，`Esc` 取消。

### 鼠标

- 点击顶部标签页、底部 `[R] Record` / `[E] Export` / `[P] Pause` / `[?] Help` 按钮。
- 点击 History 页的 `1m` / `5m` / `15m` / `1h` / `6h` / `24h` / `48h` / `All` 窗口按钮。
- 在导出页点击来源两侧的 `<` / `>`、时间×格式表格中的格子、`[ Export ]` 或 `Esc cancel`（取消）。
- 在内容区滚动：History 页切换时间窗口，其他页滚动内容。
- 鼠标捕获可能影响终端自身的文本选择；按 `m` 或启动时加 `--no-mouse` 可关闭。

最小终端尺寸为 80×24；更小时显示提示，不会 panic。

## 历史存储与导出

### 滚动 48 小时

- 程序运行期间（TUI 或无 TUI 长跑；`--demo`/`--once` 除外），每个采样自动追加到 `/tmp/macpower/history/history-<epoch-hour>.tsv`，按小时分文件，跨次启动继续追加。
- 使用紧凑 schema：保留全部 `Sample` 字段，但不写体积很大的 `rails` 列。默认 1 Hz 下约 40 MB/48 小时。
- 启动以及切换小时时会清理超过约 48 小时的文件；导出时按所选时间尺度精确截取。
- 目录权限设为 `0700`。History 页的短窗口（1m–1h）使用内存缓冲；6h、24h、48h 和 All 会从磁盘历史加载并缓存在内存中，选中后继续追加新采样。

### 录制段

- 按 `r` 开始完整 raw v2 TSV（含 `rails`）录制，默认写入 `/tmp/macpower/recordings/recording-<时间>.tsv.part`。
- 录制没有时长或大小上限；同时采样仍进入滚动 48 小时历史。
- 再按 `r` 停止：文件关闭并重命名为 `.tsv`，之后不会再写入。录制开始前的数据不会复制进该文件，仍由滚动历史提供。
- `--raw <PATH>` 仍按指定路径录制完整 raw，不自动重命名。

### 导出页

- 按 `e` 打开一页式导出页：顶部 `< 来源 >` 在最近 48 小时与各录制段之间切换；下方是时间尺度（1m / 5m / 15m / 1h / 6h / 24h / 48h / All）× 格式（PDF / TSV）表格。
- 方向键移动格子，`Tab`/`Shift-Tab` 切换来源，`Enter` 或 `[ Export ]` 按钮执行导出，`Esc` 或点击 `Esc cancel` 取消；鼠标可点击来源箭头、表格格子和按钮。
- 输出自动命名为当前目录下的 `power-report-<时间戳>.pdf` 或 `power-history-<时间戳>.tsv`。
- PDF 为三页矢量报告；所有纵轴跨越 0 的图表会绘制一条带 `0` 标签的基准线。DC 输入会在绘图前剔除适配器插拔瞬间的异常尖峰（SMC `PDTR` 偶尔会报出数百瓦）；在电池供电的历史区间按 0 显示。
- TSV 会保留所选来源的数据列：录制段包含完整 `rails`，滚动历史为紧凑列（不含 `rails`）。文件头包含 `# source` 与 `# range` 元数据。

## 数据来源与计算方式

### 整机与电源输入

整机功耗按以下优先级选择：

- **使用电池时**：电池放电功率（快速 SMC 估计优先）→ SMC `PSTR` → 固件 `SystemLoad`。
- **接交流电时**：SMC `PSTR` → SMC `PDTR` → 固件 `SystemLoad` → `n/a`。

电池放电功率是拔电状态下最接近整机功耗的物理量；`PSTR` 在电池模式下通常只覆盖部分系统电源轨，因此不再优先。

电源输入优先级：

1. SMC `PDTR`。
2. `PowerTelemetryData.SystemPowerIn`（固件遥测，和 `PDTR` 通常一致）。
3. 充电时的电池 `V × I`（粗略下限）。
4. `n/a`。

电池功率符号约定：

- 正值 = 正在充电（功率流入电池）。
- 负值 = 正在放电（电池向系统供电）。
- 界面上带 `~` 的值是 SMC 派生的快速估计；原始/JSON 中的 `battery_*_measured` 保留 IOKit 电量计读数。

### 电池功率的刷新

这台 M3 的 `AppleSmartBattery` 电量计大约每 30 秒才刷新一次 `InstantAmperage` 和 `Voltage`，直接显示会出现几十秒不变化的情况。程序因此使用快速估计：

- **放电时**：使用 SMC `PPBR`（正值）并显示为负功率；实测 `PPBR` 与电量计 `V × I` 接近，但每秒刷新。
- **充电时**：使用 `PDTR - PSTR` 估算充电功率，并用最近一次刷新后的 `V × I / (PDTR - PSTR)` 比例校准；没有校准值时使用 0.9。macOS 刚接入电源后可能长时间保持 `IsCharging=false`，此时若电池未充满且 `PDTR - PSTR` 明显大于系统消耗，也按充电处理，避免退回几十秒不刷新的电量计。
- 电流由快速功率除以最近一次电池电压得到。
- 这些值会标记为 `~`；需要原始电量计时，查看 `battery_w_measured` / `battery_a_measured` 列。

### SMC

通过 `AppleSMC` user client 读取，不需要 root：

- `PSTR`：系统功耗（W）。
- `PPBR`：电池放电功率（W）。在这台 M3 上，放电时 `PPBR ≈ |V × I|` 且每秒刷新；充电时它并不是充电功率，此时改用 `PDTR - PSTR` 估算。其他机型上的含义可能不同。
- `PDTR`：DC 输入功率（W）。
- `PBLR` / `PDBR`：屏幕背光功耗（W，若该机型提供）。该值会随亮度明显变化。

SMC key 是 4 字节 FourCC；`flt` 浮点值在 Apple Silicon 上为小端，同时兼容 Intel 的大端解释。不存在的 key 会被跳过，不影响其他数据源。

### IOReport Energy Model

通过私有 `libIOReport.dylib` 的 `Energy Model` 组读取能量计数器：

- 每个采样间隔对两次能量样本做差，再除以实际经过的时间，得到平均功率。
- 使用真实 `Instant` 间隔而不是配置的采样周期，避免调度抖动导致系统性的功率偏差。
- CPU：优先 `CPU Energy`；否则 `ECPU + PCPU`；再否则累加逐核 channel。
- 逐核：匹配 `ECPU0..N` / `PCPU0..N`，排除 `_SRAM`、`DTL`、`ECPM` 等子通道。
- GPU：优先 `GPU`；若其为 0 而 `GPU Energy` 有值，则使用更精细的 `GPU Energy`，两者不会相加。
- 其他：`ANE`、`DRAM`、`DISP + DISPEXT`。`DISP` 是 SoC 显示控制器功耗；若 SMC 提供 `PBLR` / `PDBR`，会再加上屏幕背光功耗，因此 `DISPLAY` 会随亮度变化。
- 辅助功耗轨：`ISP`、`AVE`、`MSR`、`VDEC`、`SOC_AON`、`SOC_REST`、PCIe 相关通道；`OTHER` 只是这些轨道的部分和，不是整机总功耗。
- `AMCC`、`DCS` 可能与其他通道重叠，仅在 Power 页单独显示，不并入 `OTHER`。
- 子通道（`_SRAM`、`DTL`、Power Zone Filter 等）会过滤，避免重复计数。

第一次采样没有前一个样本，组件功耗显示 `n/a`（warming up）。暂停/恢复或修改采样间隔时会重置差分基线，避免跨越空档产生虚假尖峰。

### 电池与适配器

通过 IOKit `AppleSmartBattery` 读取：

- `Voltage`（mV）、`InstantAmperage`（mA，有符号）、`Temperature`（deci-Kelvin）。
- 温度换算：`°C = raw / 10 - 273.15`。
- 健康度优先 `NominalChargeCapacity / DesignCapacity`，兜底 `AppleRawMaxCapacity / DesignCapacity`。
- `MaxCapacity` 可能是百分比而不是 mAh，代码不会把它当作容量直接相除。
- 适配器额定功率来自 `AdapterDetails`，这是额定值，不等于实际墙上功耗。
- `PowerTelemetryData` 的单位没有公开文档；仅在能推导出合理瓦数时显示，并标记为固件遥测。`SystemPowerIn` / `SystemLoad` 会在 SMC 不可用时作为整机/输入的最后兜底。
- `IOPSCopyPowerSourcesInfo` 作为电量和剩余时间的兜底。

### 指标说明

- `n/a` 表示该传感器/通道在当前 Mac 或当前 macOS 上不可用，不表示 0。
- `OTHER` 是辅助轨道的部分和，不能当作整机功耗。
- 所有数值都是估算值，适合观察趋势和相对变化，不应跨设备做绝对比较。
- 这是私有 API 采集的功耗估算，不是经过校准的功率计。

## 原始 TSV 格式

TUI 中按 `r` 开始完整原始录制，默认写入 `/tmp/macpower/recordings/recording-YYYYMMDD-HHMMSS.tsv.part`，停止后重命名为 `.tsv`；若无 `r` 则不自动录制完整 raw，但滚动 48 小时历史仍会写入。命令行 `--raw <PATH>` 按指定路径录制，不自动重命名。

文件结构：

```text
# macpower raw v2
# start=2026-11-11T14:32:05.123+08:00
# host=... chip=Apple M3 model=Mac14,x macos=14.2 build=...
# units: power=W voltage=V current=A temperature=C
# columns: timestamp_ms	iso_time	source	system_w	...
timestamp_ms	iso_time	source	system_w	...
```

列说明（缺失值为空字段，不是 0）：

| 列 | 单位 | 含义 |
| --- | --- | --- |
| `timestamp_ms` | ms | Unix 毫秒时间戳 |
| `iso_time` | — | 本地时区 RFC3339 时间 |
| `source` | — | `Battery` / `AC` / `Unknown` |
| `system_w` | W | 整机功耗最佳估计 |
| `smc_pstr_w` | W | SMC `PSTR` |
| `dc_in_w` | W | SMC `PDTR` / DC 输入 |
| `battery_w` | W | 带符号电池功率最佳值：充电为正、放电为负；可能为快速估计 |
| `battery_w_measured` | W | 电量计原始 `V × I`（约 30 秒刷新一次） |
| `battery_a_measured` | A | 电量计原始带符号电流 |
| `battery_estimated` | 0/1 | `battery_w` / `battery_a` 是否为 SMC 派生估计 |
| `battery_pct` | % | 电量 |
| `battery_v` | V | 电池电压 |
| `battery_a` | A | 带符号电池电流最佳值；可能为快速估计 |
| `battery_temp_c` | °C | 电池温度 |
| `cycle_count` | 次 | 循环次数 |
| `cpu_w` | W | CPU 总功耗 |
| `ecpu_w` | W | E 集群功耗 |
| `pcpu_w` | W | P 集群功耗 |
| `core_e_w` | W | E 核逐核功耗，分号分隔 |
| `core_p_w` | W | P 核逐核功耗，分号分隔 |
| `gpu_w` | W | GPU 功耗 |
| `ane_w` | W | ANE 功耗 |
| `dram_w` | W | DRAM 功耗 |
| `display_w` | W | 显示控制器 + 背光功耗（若 SMC 提供 `PBLR` / `PDBR`） |
| `other_w` | W | 辅助轨道部分和 |
| `rails` | W | 顶层电源轨 `name=value;name=value;...` |

`rails` 列会过滤子通道（`_SRAM`、`DTL`、Power Zone Filter 等）以保持行紧凑；`--list-sensors` 可以看到完整通道。写入时每行 flush；停止录制后文件可直接作为导出页的来源。解析器按列名读取，因此旧的 v1 文件仍然可用（新列缺失时视为空值）。滚动 48 小时历史使用同一批字段但省略 `rails` 列（`# macpower history v1` 头），以控制磁盘占用。

## PDF 报告

按 `e` 打开一页式导出页，在顶部的来源（最近 48 小时或任一录制段）与下方的“时间尺度 × 格式”表格中选择一个格子即可。方向键移动格子，`Tab` 切换来源，`Enter` 或 `[ Export ]` 导出；详见上文“历史存储与导出”。

输出默认为当前目录的 `power-report-YYYYMMDD-HHMMSS.pdf` 或 `power-history-YYYYMMDD-HHMMSS.tsv`；命令行 `--report <PATH>` 仍可指定路径，并在退出时自动生成 PDF。

报告为 A4 横向、矢量绘制、内置 Helvetica、纯 ASCII 文本，共三页：

1. 标题、系统信息、时间范围、汇总表、整机/输入/电池曲线、电池电量曲线。
2. CPU 总量与 E/P 集群曲线、GPU/ANE/DRAM/显示/其他小图、最新逐核功耗柱状图。
3. 最新快照电源轨表（功率、原始单位、来源；逐核数据见第 2 页）、总量统计与数据来源/校准注意事项。

PDF 不嵌入 CJK 字体，以避免巨大的字体依赖；UI 和报告标签均为英文。

## 兼容性与回退

- 直接在终端运行，不需要 root。
- 已验证：Apple M3、macOS 14.2、8 核（4P + 4E）。
- 目标范围：Apple Silicon M1–M4 和 Intel Mac，macOS 12+。Intel 路径做了防御性处理（SMC 大小端、缺失 IOReport 组、缺失 perflevel sysctl），但未在真实 Intel 机器上验证。
- SMC 不可用：仅使用电池 / IOReport，整机和组件数据仍可用。
- IOReport 不可用：整机和电池数据仍可用，组件面板显示不可用提示。
- 没有电池的桌面 Mac：显示 AC / DC-IN / 整机功耗，电池面板显示 `n/a`。
- 没有逐核 channel：只显示 E/P 集群总量。
- Rosetta 环境：Help 页会提示构建原生 arm64 版本。
- 在虚拟机、沙盒或未来 macOS 私有 API 变更时，`Energy Model` 组可能不存在；用 `--list-sensors` 和 `--disable` 排查。

私有 API 说明：`AppleSMC` 和 `IOReport` 不是公开 API，不适合 Mac App Store 分发；请以 Developer ID 直接分发。程序不调用 `powermetrics`，因此不需要 root，也不提供逐进程能耗。

## 测试与验证

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

测试覆盖包括：SMC 结构体布局与浮点解码、单位换算、通道分类、逐核解析、TSV 行往返、缺失值解析、历史裁剪/降采样/能量积分、48 小时滚动存储与时间过滤、PDF 三页生成/0 基准线/ASCII 清洗、应用按键与鼠标点击。

建议在本机做一次端到端验证：

```bash
./target/release/macpower --list-sensors
./target/release/macpower --once --format json
./target/release/macpower --headless --duration 5 --raw /tmp/raw.tsv --report /tmp/report.pdf
./target/release/macpower --disable ioreport --once
./target/release/macpower --disable smc --once
./target/release/macpower --disable battery --once
./target/release/macpower --demo
```

## 非目标（v1）

- 不做逐进程能耗排名。
- 不依赖 root helper，不调用 `powermetrics`。
- 不做 CPU/GPU 频率读取或风扇控制。
- 不引入数据库；TSV 是持久化格式，包括滚动 48 小时和录制段。
- 不面向 Mac App Store。

## 项目结构

```text
src/
  main.rs                 CLI、终端生命周期、无 TUI 模式
  app.rs                  应用状态、标签页、弹窗、鼠标命中区、按键处理
  model.rs                Sample / Snapshot / 电池 / 系统信息与格式化
  collectors/
    mod.rs                采样编排与整机功耗选择
    ffi/cf.rs             CoreFoundation 安全封装
    ffi/iokit.rs          IOKit / IOPS 声明
    ffi/sysctl.rs         sysctl 辅助函数
    smc.rs                AppleSMC 读取与解码
    ioreport.rs           Energy Model 订阅、差分、通道分类
    battery.rs            AppleSmartBattery + IOPS 回退
    sysinfo.rs            芯片 / macOS / 内存 / 启动时间
  history/
    mod.rs                环形缓冲、降采样、统计与能量积分
    raw.rs                TSV 录制、紧凑历史编码与解析
    store.rs              /tmp 滚动 48 小时历史与录制段管理
    pdf.rs                矢量 PDF 报告
  ui/
    mod.rs                根布局、标签、状态栏、弹窗
    overview.rs           Overview 页
    cpu.rs                CPU 页
    power.rs              Power 页
    history.rs            History 页
    help.rs               Help 页
    theme.rs              深色终端配色 / 单色模式
    widgets/graph.rs      多序列盲文图、ASCII 图、sparkline
    widgets/bar.rs        组件条与电池仪表
```
