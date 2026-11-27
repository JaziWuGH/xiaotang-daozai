# 岛仔插件运行环境实测

**评审建议：首发优先考虑 QuickJS。** 岛仔已经随应用发布 QuickJS，作者可以用文本编辑器改脚本；WASM 的优势是 Rust 业务逻辑可以继续用 Rust 编写，指令额度也更容易重复测试。本次只验证同一款倒计时。一次加载十个实例的热中位数为 WASM 4.12 ms、QuickJS 12.28 ms；两者的体积增量分别为 1.163 MiB 和 0.152 MiB。如果首发作者主要是已有 Rust 模块的维护者，值得拿他们实际要迁移的模块再比较一次。最终选型由小唐决定。

## 同一款插件，两套实现

用户在「待办与置物」设置标题和目标日期，展开岛后看到“距离纪念日还有 N 天”。宿主提供按本地日历计算的目标日和今天的日期序号，插件算差值，先读上次显示值再存本次结果。宿主绘制卡片、校验配置并原子写盘。两个引擎各用自己的数据目录。对照样本使用同一套绘制、设置控件、刷新节奏和存储桥接。

| 真实窗口截图 | QuickJS | WASM |
| --- | --- | --- |
| 同一程序的独立验证窗口 | ![QuickJS 实际运行的倒计时和设置](evidence/js.png) | ![WASM 实际运行的倒计时和设置](evidence/wasm.png) |

截图由 eframe 渲染并读回帧缓冲。验证窗口调用与岛内实验入口相同的 `View::draw`、`View::settings`，独立运行便于复测。正常岛体的入口是 `--sdk-live`，在本次验证中临时使用待办页和对应设置页；正式模块注册、集市和通用设置渲染属于后续 SDK 工作。

## 同机对比

| 维度 | QuickJS · rquickjs 0.8.1 | WASM · wasmi 0.46.0 |
| --- | --- | --- |
| release 程序（默认 22,366,208 B） | 22,525,440 B；增加 **0.152 MiB** | 23,585,792 B；增加 **1.163 MiB** |
| 加载 1 个（首轮 / 热中位 / 热 P95） | 20.95 / **1.24** / 1.69 ms | 14.43 / **0.41** / 0.88 ms |
| 加载 10 个（首轮 / 热中位 / 热 P95） | 50.28 / **12.28** / 13.86 ms | 31.89 / **4.12** / 5.84 ms |
| 11 月 17 日旧验证窗口的进程 CPU（复测见下文） | 4015.62 ms / 30.02 s（单核 13.378%） | 937.50 ms / 30.01 s（单核 3.124%） |
| 同一采样窗口的进程 CPU 周期数 | 9,930,499,926 周期 | 2,406,870,340 周期 |
| 进程工作集（中位 / 峰值） | 79.66 / 83.92 MiB | 80.24 / 80.87 MiB |
| 只刷新数据，无窗口（单核 CPU / 工作集中位） | 0.00 ms / 30.01 s；10.99 MiB | 0.00 ms / 30.01 s；11.07 MiB |
| 死循环 / 异常 / 超内存后 | 3/3 中止；同一实例均能再次刷新，死循环在 30.635 ms 返回 | 3/3 中止；同一实例均能再次刷新，死循环在 0.125 ms 返回 |
| 权限 | 未声明请求 -1、已声明 204；文件模块和直连网络入口不可用 | 未声明请求 -1、已声明 204；未注册的 WASI 文件导入链接失败 |
| 作者代码行数（倒计时 / 含故障用例） | 6 / 14 行；直接运行脚本 | 19 / 38 行；Rust 编译为 335 字节 WASM |
| 保存一行到出现新画面（单次） | 519 ms，含准备 0.9 ms | 2551 ms，含准备 2068.9 ms |
| 从本体抽取数字范围（原控件 1 行） | 新脚本 5 行代码；3/3 输入符合原范围 | 新 Rust 源码 16 行；3/3 输入符合原范围 |
| 依赖和 CI 实际运行 | 沿用既有引擎；本次跑过脚本、Windows 宿主编译、截图 | 锁文件新增可选解释器依赖树；本次额外安装 WASM 编译目标，编译、链接并运行两个插件样本 |

测量日期为 2026 年 11 月 17 日。基线提交 `efb8266`，Windows x64、Intel Core i5-12450H、12 个逻辑处理器、16 GB 内存。三份 exe 均用 Rust 1.96.1、同一份锁文件、同一 release 配置（LTO、单代码生成单元、剥离符号）。增量包含共用的验证窗口、命令行和测量代码。1 MiB = 1,048,576 字节。完整数据见[测量结果](evidence/results.json)、[QuickJS 加载原始样本](evidence/js-load.json)、[WASM 加载原始样本](evidence/wasm-load.json)。

加载计时包括读插件与配置、实例化、第一次回调和存储；10 个插件在同一进程内保持 10 份实例，同时存活。每组先跑一次，再重复 20 次取中位数与第 95 百分位。第一轮包含文件系统缓存状态的影响；时间从宿主开始装插件算起。运行开销按可见验证窗口每秒刷新一张卡测量，预热 3 秒后每秒取一次工作集，共 30 次。CPU 百分比按一个逻辑核折算，包含宿主、窗口事件、绘制和引擎，不能据一次低负载采样判断引擎快慢。作为对照，无窗口模式每秒调用同一条刷新路径，共完成 35 次，取中间 30 秒的 CPU 和工作集；Windows 进程 CPU 计时可能舍入到 0。修改时延从保存一行“天数 +1”算起，含必要的编译、重新启动验证窗口、读回并保存画面；另用独立进程核对天数确实增加。这里没有实现开发者模式的自动热重载。

## 11 月 19 日复测：窗口为什么会出现四倍差距

**原表比较的是两个窗口进程在各自 30 秒内的总 CPU。用这个数字估算插件执行成本，会把桌面触发的重绘也算给引擎。** 旧预览只在打开时创建一次引擎；之后每次绘制先判断是否到了刷新时间，真正到期才调用插件。正常岛体的本地开发包则在每次刷新时新建环境并在后台执行。两条路径见[预览](../../spike/ui.rs)和[本地插件](../../spike/local.rs)。

先重跑原来的两个 release 文件，沿用预热 3 秒、取 30 秒的口径，按 QuickJS、WASM、WASM、QuickJS 顺序执行，单核 CPU 依次为 0.260%、0.156%、0.104%、0.208%。随后将两种引擎编入**同一个程序**，给窗口加上绘制次数、实际调用次数以及刷新函数总耗时三个计数。每个新窗口首次报告计数后再预热 5 秒，采 20 次，每次间隔约 1 秒。

| 窗口条件 | 引擎 | 实测绘制帧数 | 实际调用次数 | 全部刷新累计耗时 | 进程单核 CPU |
| --- | --- | ---: | ---: | ---: | ---: |
| 原垂直同步，第 1 轮 | QuickJS | 136 | 12 | 0.234 ms | 9.592% |
| 原垂直同步，第 1 轮 | WASM | 22 | 11 | 0.175 ms | 0.149% |
| 原垂直同步，交换顺序 | WASM | 22 | 11 | 0.173 ms | 0.000%¹ |
| 原垂直同步，交换顺序 | QuickJS | 22 | 11 | 0.254 ms | 0.682% |
| 与正常岛体一致，关闭垂直同步 | QuickJS | 22 | 11 | 0.261 ms | 0.376% |
| 同上 | WASM | 22 | 11 | 0.206 ms | 0.301% |
| 关闭垂直同步，每 16 ms 请求重绘 | QuickJS | 29,943 | 21 | 0.423 ms | 129.457% |
| 同上 | WASM | 30,841 | 22 | 0.373 ms | 129.546% |

¹ Windows 进程时间按时间片计量，短区间可能读到零。表中按一个逻辑核折算；多个线程合计可以超过 100%。帧数与调用数来自窗口约每秒发布的计数，起止点与进程采样可能相差一个发布间隔。

同一个 QuickJS 程序在原窗口设置下，两轮用掉的进程 CPU 时间分别为 1921.875 ms 和 140.625 ms，脚本刷新总耗时却相差不到 0.03 ms。第一轮实际多画 114 帧，窗口有 6 个采样点处于后台。再只加快重绘，两种引擎都升到约 129.5%，脚本仍约每秒调用一次。这里 16 ms 是请求值，egui 会先扣掉预计的绘制时间，短请求可触发立即重画。故意加密重绘的两行仅用于验证原因。[egui 的重绘调度](../../vendor/egui/src/context.rs)

复测定位到了**窗口渲染负载和采样条件**，排除了“QuickJS 每帧重新解析或执行倒计时”对这个差距的解释。验证窗口已与 Windows 正常岛体采用相同的垂直同步设置。11 月 17 日没有记录当时的帧数、焦点和驱动等待，无法回推当日究竟是哪次窗口事件造成 13.378%，也没有证据把整个差额都归给垂直同步。

### 正常岛体装十个插件

这里启动的是正常岛体，使用同一份带两种引擎的 release 程序和独立的数据目录。为保留日常使用的岛仔，测量实例跳过单实例检查、托盘图标和安装体积登记。每个已启用的包有自己的 ID、设置和保存记录，使用发薪倒计时的真实脚本，另在存储里记下成功刷新次数。原生窗口、模块检查、后台运行插件、绘制和系统采样均使用正常入口。

| 开启的倒计时 | 采样时长 | 单核 CPU | 工作集中位 / 峰值 | 各插件成功调用次数（开始 → 结束） |
| --- | ---: | ---: | ---: | --- |
| 0 个 | 72.44 s | 11.540% | 129.71 / 133.75 MiB | 无插件 |
| 10 个 | 70.12 s | 11.276% | 131.23 / 133.89 MiB | 每个均为 1 → 2 |
| 10 个 | 71.55 s | 11.858% | 131.13 / 135.04 MiB | 每个均为 1 → 2 |
| 0 个 | 70.91 s | 11.215% | 129.96 / 132.52 MiB | 无插件 |

十个倒计时的两轮工作集中位为 **131.13–131.23 MiB**，比两轮空岛的中位约多 1.2–1.5 MiB。十插件进程的单核 CPU 为 **11.276%–11.858%**。四轮都在可见的 Windows 桌面测量；最后三轮都在 70 个采样点检测到鼠标位于岛体窗口范围内。进程值包含岛体绘制及其他后台工作，两个小范围样本不足以给出每个插件的固定 CPU 成本。本次负载是每分钟更新一次的文字卡片，实时歌词或外部设备查询需要各自实测。[逐轮数据、桌面状态和程序哈希](evidence/cpu-review.json) · [复测脚本](../../spike/measure_cpu.py)

## 故障、权限与重启

| 注入的情况 | QuickJS 实际返回 | WASM 实际返回 |
| --- | --- | --- |
| 死循环 | `Err(Exception)`；30.635 ms | `Err(Error { kind: TrapCode(OutOfFuel) })`；0.125 ms |
| 异常 | `Err(Exception)`；0.215 ms | `Err(Error { kind: TrapCode(UnreachableCodeReached) })`；0.003 ms |
| 内存上限 | `Err(Exception)`；0.246 ms | `Err(Error { kind: TrapCode(GrowthOperationLimited) })`；0.009 ms |
| 未声明联网 | `Ok(-1)`；0.816 ms | `Ok(-1)`；0.054 ms |
| 已声明模拟传输 | `Ok(204)`；0.007 ms | `Ok(204)`；0.019 ms |
| 直接文件访问 | `Exception generated by QuickJS` | `cannot find definition for import wasi_snapshot_preview1::path_open with type Func(FuncType { core: FuncType { params: [], results: [] } })` |

JS 为每个实例设置 4 MiB 堆、128 KiB 栈和 30 ms 中断期限。WASM 每次调用补充 100,000 单位指令额度，单实例限制一块 4 MiB 线性内存、一张最多 1,024 项的表，超限增长直接陷入错误。WASM 指令额度与墙钟时间没有固定换算。每次执行故障、拒绝请求或接受模拟请求后，测试都立即在**原宿主进程、原实例**刷新一次，要求它提交有效天数。

允许域名为 `allowed.example`；宿主在解析 HTTPS 地址、核对主机和端口后才进入内存传输端。未声明域名的调度次数增量必须为 0，允许域名必须为 1。文件探针分别尝试使用 JS 的文件模块、导入 WASI 文件函数；两者均未向插件提供操作系统文件句柄。本次没有发公网请求，也没有实现重定向、DNS 地址复核或生产文件选择器。故障演示覆盖脚本异常、指令耗尽和受限内存分配。引擎自身的原生崩溃、解析器漏洞或阻塞的宿主函数仍需独立进程与宿主侧限时兜底。

两套 release 程序各启动两次。QuickJS 首次返回 38 天、上次值为空，重启读回“旅行”、2026-12-25、上次 38 天；WASM 首次返回 38 天、上次值为空，重启读回“旅行”、2026-12-25、上次 38 天。

## 作者与维护者要改什么

- **倒计时源码。** [JavaScript 插件](../../spike/plugins/countdown.js)和 [Rust/WASM 插件](../../spike/plugins/countdown.rs)共享一份[样例配置](../../spike/plugins/settings.json)。显示文案、标题/日期输入和保存由宿主提供。实测行数按仓库内非空源码行计算；故障探针单列，Rust 计入无需标准库的入口和导入声明。
- **从健康设置拆一段。** 取现有 `src/extras.rs` 的喝水提醒数字控件，原调用 1 行，范围 5–180 分钟。生产设计把范围、单位和设置归属放到清单，宿主继续画原控件。本验证把日期差当作合成分钟输入，分别执行[脚本版](../../spike/migration/interval.js)和 [Rust 版](../../spike/migration/interval.rs)，测上下界与原有存储接口。两者都实测 4 → 5、45 → 45、181 → 180。抽样覆盖范围规则，完整喝水模块的通知排程、UI 和后台恢复未迁移。
- **依赖维护。** 现有中转站脚本使用 `rquickjs 0.8.1`，验证沿用锁文件。WASM 使用可选 `wasmi 0.46.0`，关闭默认特性，仅启用 std，不链接 WASI。查询时两项目均有 2026 年发行记录：rquickjs 0.14.0（9 月 18 日）、wasmi 2.0.0（9 月 1 日）。发布活动不能证明当前锁定版本仍获安全修复；正式接收社区代码前需评估升级，并复跑旧插件原包。上游记录见 [rquickjs](https://crates.io/crates/rquickjs/versions) 和 [wasmi](https://crates.io/crates/wasmi/versions)。
- **CI 与普通作者。** 两套宿主均用项目原有 Windows Rust/MSVC release 构建。JS 作者改文本即可；Rust 插件作者或重建其制品的 CI 需额外安装 `wasm32-unknown-unknown` 并将源编为字节码。普通用户安装现成 WASM 包无需此工具链。测量脚本另需 Python、psutil 及能创建原生窗口的 Windows 会话。宿主 SDK、权限和旧包回归仍是两种路线共有的维护工作。

## 本地复验

在此分支仓库根目录打开 PowerShell。默认构建不打开验证功能；下面分别构建并保留三份完整程序，每份使用同一份 Cargo.lock。

```powershell
$tc = '+stable-x86_64-pc-windows-msvc'
$triple = 'x86_64-pc-windows-msvc'
rustup target add wasm32-unknown-unknown --toolchain stable-x86_64-pc-windows-msvc
rustc $tc --target wasm32-unknown-unknown --crate-type cdylib -C opt-level=s -C panic=abort -C link-arg=-zstack-size=65536 -C strip=symbols spike/plugins/countdown.rs -o spike/plugins/countdown.wasm
foreach ($kind in 'base','js','wasm') {
    $features = @()
    if ($kind -ne 'base') { $features = @('--features', "sdk-$kind") }
    powershell -NoProfile -ExecutionPolicy Bypass -File spike/cargo-serial.ps1 build --locked --release --bin daozai --target $triple @features
    if ($LASTEXITCODE -ne 0) { throw "构建失败：$kind" }
    Copy-Item "target/$triple/release/daozai.exe" "target/spike-$kind.exe"
}
powershell -NoProfile -ExecutionPolicy Bypass -File spike/cargo-serial.ps1 test --locked --lib --target $triple -- --test-threads=1
```

- **试用设置与重启。** 运行 `./target/spike-js.exe --sdk-spike js ui`，把标题改为“发薪”，填入目标日期，点保存。关闭窗口后重开，可核对标题、日期和上次显示的天数。将文件名与参数里的 `js` 一起改成 `wasm` 即可对照。体验完整岛体时可在退出既有岛仔实例后运行 `./target/spike-js.exe --sdk-live js` 或 `./target/spike-wasm.exe --sdk-live wasm`；展开待办页看卡片，从待办对应设置调参数。
- **复测与截图。** 安装 Python 的 `psutil` 后运行 `python -X utf8 spike/measure.py target/review-01`。每次指定一个新目录，脚本复制样例供修改，测试设置、截图和原始数据均写到该目录。两次手工打开验证窗口默认共用临时目录下的 `daozai-sdk-spike/<引擎>/ui`；可用 `DAOZAI_SPIKE_DATA` 指向全新目录，`DAOZAI_SPIKE_PLUGINS` 指向自己的样例复制件。
- **回归。** 默认 release 构建成功；按任务指定命令运行现有库测试，1280 项通过、0 项失败，另有仓库原标记忽略的 129 项。SDK 验证单独运行，不计入这 1280 项。

正式权限和版本约定、建议值及待决定事项见 [SDK v1 设计评审稿](sdk-v1-design.md)。

### 重做本轮 CPU 采样

从仓库根目录运行以下命令。第二个参数给一个全新的目录。脚本按顺序启动测试窗口和正常岛体，用独立数据目录，结束时只关闭自己启动的进程。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File spike/cargo-serial.ps1 build --locked --release --features sdk-js,sdk-wasm --bin daozai --target x86_64-pc-windows-msvc
python -X utf8 spike/measure_cpu.py target/x86_64-pc-windows-msvc/release/daozai.exe target/cpu-review-new
```

## 升级 rquickjs 0.8.1 到 0.14.0 的影响

rquickjs 是岛仔用 Rust 调用 QuickJS 的库。核对了 0.14.0 的上游源码，内存限制、栈限制、中断检查以及把结果转回 JSON 的接口仍在。需要一起核对三处调用：本地插件的每次全新执行、运行环境验证里的宿主函数，以及正式版中转站额度脚本的响应提取。升级会同时替换底层 C 引擎、Windows 构建依赖和锁文件。现有接口名字相同，只能减少改代码的范围，异常、超时、内存限额和字符转换仍需执行验证。[上游运行时实现](https://docs.rs/rquickjs-core/0.14.0/src/rquickjs_core/runtime/base.rs.html) · [上下文和 JSON 转换](https://docs.rs/rquickjs-core/0.14.0/src/rquickjs_core/context/ctx.rs.html)

建议按中等改造成本、高验证优先级安排。正式接收社区代码前，在单独分支升级，复跑两个官方原包、额度提取、死循环、内存超限和拒绝访问的用例，再比较默认版体积、冷启动和十插件负载。通过后切换并保留上一个可用版本以便回退。本轮依赖仍锁定 0.8.1，0.14.0 的编译和运行兼容性留给升级分支实测。
