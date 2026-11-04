<div align="center">
<img src="assets/daozai-logo.png" width="112" alt="岛仔" />

# 岛仔 · Daozai

**让重要的信息，轻轻出现在屏幕顶部。**

Windows 桌面伙伴 · 音乐与歌词 · 邮件验证码 · 日程提醒 · AI 额度

[![Release](https://img.shields.io/github/v/release/Renly1994/xiaotang-daozai?color=6d65ff&label=下载)](https://github.com/Renly1994/xiaotang-daozai/releases/latest)
[![Windows](https://img.shields.io/badge/Windows-10%20%2F%2011-0078D4)](#安装与更新)
[![License](https://img.shields.io/badge/License-MIT-3cbf91)](LICENSE)
[![CI](https://github.com/Renly1994/xiaotang-daozai/actions/workflows/ci.yml/badge.svg)](https://github.com/Renly1994/xiaotang-daozai/actions/workflows/ci.yml)

[下载 Windows 版](https://github.com/Renly1994/xiaotang-daozai/releases/latest) · [使用指南](docs/USAGE.md) · [更新记录](CHANGELOG.md) · [反馈问题](https://github.com/Renly1994/xiaotang-daozai/issues)

</div>

![收起状态：贴在屏幕顶部的刘海](docs/images/showcase-hero.png)

## 一个小岛，装下桌面上的小事

岛仔贴在屏幕顶部，收起时是一条小刘海。鼠标靠近时展开，离开时收起；需要处理的信息出现时再提醒你。它由 Rust 和 egui 构建，不依赖 Electron。

展开后，触控板双指左右滑动可以翻页：手指向左是下一页，向右是上一页。竖向滚轮和点击保持原来的行为。

| 功能 | 可以做什么 |
| --- | --- |
| 桌面伙伴 | 会眨眼、打盹、喝水，也会拿起麦克风跟着音乐唱；可切换极简模式 |
| 音乐与歌词 | 读取 Windows 媒体会话，控制播放、切歌、进度；支持岛上单行歌词 |
| 专注与饮水 | 专注／休息计时、饮水目标、提醒时段、快捷记录与撤销 |
| 文件置物架 | 拖入临时收集，打开或定位原文件，拖出到其他应用；移除记录不删除原文件 |
| 待办 | 添加、编辑、完成任务；用日期和时间选择器设置可选提醒 |
| 邮件 | 多账号收信、验证码来源识别与点击复制、正文阅读、默认浏览器打开链接 |
| 日程 | 月历和当日安排；合并工作日历与邮件邀请，支持全天、跨天、重复日程及单次修改／取消 |
| AI 额度 | Codex、Claude、Kimi、Grok Bot、Gemini；多账号、常驻圆环、重置时间和自选配色 |
| 系统提示 | 电源、电量、网络、音量、大写锁定、输入语言和蓝牙设备状态 |

### 专注工作，也记得休息和喝水

开始一段专注时间，在专注与休息之间切换；饮水面板记录当天进度，可以设置目标和提醒时段。

![鼠标靠近后展开专注面板](docs/images/showcase-daily.png)

### 待办随手记，文件临时放

把待办加到屏幕顶部，完成后勾选；需要提醒的事项可以单独设置日期和时间。置物架则用来临时收集文件，支持打开、定位原文件和拖出到其他应用。

![从顶部展开待办清单](docs/images/showcase-work.png)

### AI 额度，抬眼就能看见

收起时用圆环显示额度，展开后查看账号与重置时间。支持多账号，可选择常驻展示的服务和配色。

各服务与订阅的连接方式及可用性见下方「连接哪些服务」。

### 邮件和日程，放在同一条时间线上

![日程提醒在屏幕顶部出现](docs/images/showcase-calendar.png)

- 正式日历邀请自动进入日程；普通邮件正文出现日期不会自动创建日程。
- 可忽略某条邮件日程，停止显示和提醒；邮件保留，可从详情恢复。
- 滚轮只滚动下方列表；大卡片保持当前选择，点击其他日程才切换详情。
- 同一场日程使用 UID 与开始时间去重，工作日历和邮件来源可以同时标识。

以上为基于演示界面制作的 AI 产品展示图，展示收起、展开与提醒状态，界面细节以实际版本为准。[查看原始界面截图](docs/images)。

## 安装与更新

1. 前往 [Releases](https://github.com/Renly1994/xiaotang-daozai/releases/latest)，下载安装版 `Daozai-Setup-v0.6.50-x64.exe`。
2. 双击安装器，可修改安装位置、选择是否创建桌面快捷方式和开机自启，再点击「一键安装」。桌面快捷方式默认勾选，首次安装默认不开机自启；无需解压。
3. 安装完成后打开「岛仔」，也可从桌面快捷方式启动。在托盘菜单打开设置，选择需要的功能。

需要免安装使用时，下载 `Daozai-v0.6.50-windows-x64.zip` 并完整解压，运行其中的 `daozai.exe`。程序仍默认使用当前用户的应用数据目录。

在「设置 → 关于」点击「检查更新」，可查看最新正式版及更新说明，选择「下载并安装」「浏览器下载」或「稍后」。应用内下载通过校验后会重启更新；也可在发布页下载后再次安装，原设置和记录会保留。卸载可通过 Windows「已安装的应用」完成；卸载同样保留用户数据。详细操作见 [使用指南](docs/USAGE.md)。

**系统要求：** Windows 10／11 x64，支持 OpenGL 的显卡驱动。Windows 11 已进行本机实测；Windows 10、不同显卡及多显示器组合的兼容性仍需更多反馈。暂不提供 macOS、Linux 或 ARM64 成品包。

**校验下载：** Release 附带 `SHA256SUMS.txt`，可用 `Get-FileHash -Algorithm SHA256` 比对。当前程序未做商业代码签名，Windows 可能提示未知发布者；请核对仓库与文件校验值后再决定运行。

## 连接哪些服务

| 类型 | 当前入口 | 说明 |
| --- | --- | --- |
| 邮箱 | QQ、163、126、腾讯企业邮、iCloud、Gmail | IMAP；使用服务商要求的授权码或 App 专用密码 |
| 工作日历 | 飞书、钉钉、企业微信 | CalDAV；需要平台提供的独立日历账号／密码，安装桌面端本身不等于授权 |
| AI 额度 | Codex、Claude、Kimi、Grok Bot、Gemini | 各服务的本机登录引用或独立连接方式；不同账号与订阅的可用性可能不同 |

QQ 收信、邮件日程和飞书个人日历已进行真实账号测试；钉钉、企业微信已验证接入代码与界面，但尚未在本轮完成真实账号同步验收。不要把图标或模拟日程预览视为实连保证。

## 数据留在什么地方

- 设置、待办、饮水记录、置物路径和连接元数据保存在 `%APPDATA%\Daozai`。
- 邮箱和日历密码保存在 Windows 凭据管理器；Gemini 会话使用 Windows DPAPI 加密。
- 读取已有 AI 客户端登录时不回写原客户端配置；不要把授权码、Cookie 或登录 JSON 上传到 Issue。
- 邮箱读取收件箱，不读取垃圾邮件文件夹；邮件正文在当前进程中用于展示和识别。
- 开启对应功能后会访问服务商接口和歌词服务，不等于完全离线软件。

完整说明见 [隐私与安全](docs/PRIVACY.md)。

## 从源码构建

需要 Rust stable、MSVC C++ 构建工具和 Windows SDK。开发时建议使用较新的 stable Rust。

```powershell
rustup default stable-x86_64-pc-windows-msvc
cargo build --locked --release --bin daozai
.\target\release\daozai.exe
```

### 用这份构建替换已经安装的岛仔

默认安装位置是 `%LOCALAPPDATA%\Programs\Daozai\daozai.exe`（也就是 `C:\Users\<用户名>\AppData\Local\Programs\Daozai\daozai.exe`）。如果安装时改过目录，以「设置 → 应用 → 已安装的应用 → 岛仔」里的安装位置为准。设置和记录在 `%APPDATA%\Daozai`，替换程序不会动它们。

1. 托盘图标上退出岛仔。程序有单实例互斥体，不退出就覆盖，新文件可能写不进去，旧进程也不会换成新的。
2. 在本仓库执行上面的 `cargo build --locked --release --bin daozai`。
3. 把 `target\release\daozai.exe` 复制到安装目录，覆盖原来的 `daozai.exe`。PowerShell 示例（安装在默认位置时）：

```powershell
Copy-Item -Force .\target\release\daozai.exe "$env:LOCALAPPDATA\Programs\Daozai\daozai.exe"
& "$env:LOCALAPPDATA\Programs\Daozai\daozai.exe"
```

免安装 ZIP 则覆盖解压目录里的 `daozai.exe`。不要只复制 exe 却丢掉它旁边的安装器脚本；日常替换运行中的程序，只覆盖 `daozai.exe` 即可。

「设置 → 关于」里的检查更新仍指向上游 [Renly1994/xiaotang-daozai](https://github.com/Renly1994/xiaotang-daozai)。从那里下载安装包会覆盖这份 exe。要用这次的展开、触控板翻页和播放控制改动，就保留这里编出来的 `daozai.exe`。

## 展开卡顿与触控板翻页

这两处是相对上游 v0.6.50 的改动。同一分支还带上播放控制：后台切换播放模式、音乐卡片热区、开关式激活、播放器音量滑条，以及没在放歌时点封面启动本机播放器。

**展开掉帧。** 鼠标移进刘海时，每一次 `CursorMoved` 都会要求立刻重绘。原先在界面线程里 `sleep` 把帧率压到 60，这段睡眠发生在窗口过程里，Windows 派不出 `WM_NCHITTEST`，光标停在刘海上就会一顿一顿。与此同时，展开弹簧的过冲让窗口区域高度每帧都变，`SetWindowRgn(..., redraw=true)` 会同步擦除透明窗口。现在帧间隔改由事件循环推迟下一次重绘（仍然大约 60 帧，OpenGL 垂直同步保持关闭，避开部分 Intel 驱动在 `wglSwapBuffers` 里的忙等）；区域在形体真正长过收起裁剪区之后一次性放到目标尺寸，过冲期间不再改，并且不再要求 GDI 同步重画。

**触控板和触摸屏。** 展开并且面板基本到位之后可以左右滑切页。触控板双指走横向滚轮（Windows 的 `WM_MOUSEHWHEEL`）；触摸屏单指走指针接触（`WM_POINTER` / `WM_TOUCH` → egui 的 `Event::Touch`），不依赖滚轮。手指向左是下一页，向右是上一页。winit 会把触控板的横向滚轮取反，所以正的滚轮增量对应向左；触摸点没有这层取反，按手指实际移动的方向翻页。累计大约 24 点才翻一页，停顿、松手或反向会丢掉余量，连续滑动 0.2 秒内只翻一页。同一帧里竖向分量更大时仍走原来的滚轮：列表内部先滚动，顶栏或列表到边才翻页。点击、轻点和左右方向键不变；普通鼠标拖动不翻页。手指按在岛上时，即使系统光标还停在别处，岛也保持展开。若在 Windows 触控板设置里打开了「反转滚动方向」，触控板左右滑会跟着那项设置反过来，触摸屏仍跟手指。同一帧里如果已经有滚轮，就只走滚轮，避免双指触控板和触摸各翻一次。

### 怎么验证

Linux 上可以编译并跑逻辑测试（不包含 Windows 窗口本身）：

```bash
cargo test --locked --all-targets --features record -- --test-threads=1
```

相关用例：`expand_region_stays_put_through_the_overshoot`、`trackpad_swipe_turns_pages_in_the_finger_direction_and_ignores_jitter`、`horizontal_swipe_changes_pages_over_a_list_without_scrolling_it`、`click_still_changes_pages_after_a_small_swipe`、`touch_swipe_turns_pages_in_the_finger_direction_and_ignores_jitter`、`touch_swipe_changes_pages_over_a_list_without_scrolling_it`、`click_still_changes_pages_after_a_small_touch_swipe`、`touch_lift_discards_partial_swipe_and_mouse_drag_does_not_turn_pages`、`touch_swipe_keeps_the_island_open_when_the_mouse_cursor_is_elsewhere`，以及原有的 `wheel_*` 滚轮用例。

`mail_report_save_and_failure_paths` 在 Linux 上会失败：非 Windows 的凭据保存直接返回「仅 Windows 支持凭据管理器」，账号不会写入。这是上游行为，不是这次改动引入的。其余库测试在 Linux 上通过。

Windows 上替换安装后看这几件事：鼠标移上刘海再停住，展开过程应连续，桌面光标不该跟着一卡一卡；展开后触控板双指左滑到下一页、右滑回上一页；触摸屏单指同样左滑到下一页、右滑回上一页，竖着滑仍滚动列表；点底部圆点仍然能切页。本环境是 Linux，没有在真机 Windows 笔记本上录到帧时间，触摸这条靠注入 `Event::Touch` 的用例覆盖。

运行测试及打包：

```powershell
cargo test --locked --all-targets --features record -- --test-threads=1
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\package.ps1
```

测试按单线程运行，以隔离使用进程环境变量的数据目录测试。低内存设备可设置 `$env:CARGO_BUILD_JOBS = "1"`。打包需要 NSIS 3，可通过 `-NsisCompiler` 指定 `makensis.exe`。发布脚本会检查程序版本和运行库依赖，生成单文件 EXE 安装器、精简免安装 ZIP 和 SHA-256 校验文件。

项目结构、界面预览与贡献方式见 [开发指南](CONTRIBUTING.md)。

## 开源与致谢

本仓库的代码从上游 [Renly1994/xiaotang-daozai](https://github.com/Renly1994/xiaotang-daozai) v0.6.50（`b93dd84fc9bdb95978fef72ffb7068bdf47ac5a9`）导入，岛仔原创代码仍采用 [MIT License](LICENSE)。Rust 依赖、vendored 代码、第三方图标及品牌标识沿用各自的许可或权利声明；MIT 不授予这些服务的商标使用权，也不代表官方合作或认证。详见 [第三方声明](THIRD_PARTY_NOTICES.txt) 和 [依赖说明](docs/DEPENDENCIES.md)。

感谢 egui／eframe、Rust 社区，以及提供参考与素材的开源项目。
