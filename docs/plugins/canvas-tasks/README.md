# 自绘画布与首批官方插件：任务说明

插件 SDK 下一步开放「自绘画布」。插件不再只能从固定组件里挑，而是用一份 JSON 描述要画的图形，岛仔负责绘制、缩放、主题、帧率和安全。首批五个官方插件全部用画布实现，同时作为社区作者的样例。

设计稿在 [ui/](ui/) 目录，用浏览器直接打开：

- `00-开放方案.html`：画布方案总览。
- `P1`–`P5`：五个插件。每张稿左边是展开页（放大 1.5 倍显示），下面是常驻条和其他状态，右边是设计说明和实现方式。

在线版是画布「岛仔官方插件 UI 稿」的「理想版」页，两者内容一致；有出入时以小唐的最新说明为准。

## 原则

- 画布是数据，不是代码。插件每次刷新返回一份图形描述，不能每帧运行脚本。
- 会动的内容（时间、跳动的金额、倒计时、指针）用「绑定」交给岛仔按秒或按分钟计算。
- 标准组件全部保留。画布可以作为状态页里的一个组件，也可以占满整页。
- 不强制打码。隐私交给用户，用户可以用岛仔已有的「截图和录屏中隐藏」等设置。
- 字体默认用岛仔自带的。用户安装字体包后，画布可以按名字引用这个字体，缺字时回退到岛仔字体。
- 动画不能让岛一直重绘。静止时不出帧；有动画时，兼容渲染下要按交互帧交付，参考 `src/main.rs` 里 `page_animating` 的处理。

## 阅读顺序

1. 本文：原则、分支和交付要求。
2. [WORK-PLAN.md](WORK-PLAN.md)：按宿主后端、宿主前端、投稿校验、插件逻辑、插件画面、数据分层的分工和四周排期。**五个插件从第一天起同时开工。**
3. 自己负责的任务说明，以及 `ui/` 里对应的设计稿。

## 任务

| 任务 | 仓库 | 分支 | 什么时候开工 |
| --- | --- | --- | --- |
| [A：画布核心](A-canvas-core.md) | 岛仔 | `feat/plugin-canvas` | 立即 |
| [B：实时绑定、动画与交互](B-canvas-live.md) | 岛仔 | `feat/plugin-canvas-live` | A 之后 |
| [C：常驻小画布、字体包与设置补项](C-canvas-extras.md) | 岛仔 | `feat/plugin-canvas-extras` | A 之后，与 B 并行 |
| [P0：投稿校验支持新插件](P0-submission.md) | 插件仓 | `feat/submission-status-canvas` | 立即 |
| [P1：便签本](P1-notes.md) | 插件仓 | `feat/official-notes` | 逻辑层立即；画面层 A 之后 |
| [P2：现货金价](P2-gold.md) | 插件仓 | `feat/official-gold` | 同上 |
| [P3：实时工资](P3-live-pay.md) | 插件仓 | `feat/official-live-pay` | 同上 |
| [P4：世界时钟](P4-world-clock.md) | 插件仓 | `feat/official-world-clock` | 同上 |
| [P5：日子](P5-days.md) | 插件仓 | `feat/official-days` | 同上 |

## 分支与工作树

- 岛仔任务从本地 `main` 开分支。工作树放在 `D:\A-TangHao\REA\Windows小工具\01-专项工程\<功能中文名>`，例如 `插件自绘画布`。
- B 和 C 从 A 的分支开出。
- 插件任务在插件仓 `D:\A-TangHao\REA\Windows小工具\01-专项工程\岛仔插件仓` 开分支，工作树放在同级目录，例如 `官方插件-便签本`。
- 编译和测试按仓库根目录 AGENTS.md 的「本机编译」规则排队执行。命令统一用 `spike/cargo-serial.ps1`。
- 做完后提交到自己的分支，不合进 `main` 或 `test/acceptance`，也不推送。由小唐转交验收。

## 交付时附上

1. 分支名和最后一个提交号。
2. 下面这条命令的结果，以及任务里另列的定向测试结果：

   ```powershell
   powershell -NoProfile -ExecutionPolicy Bypass -File spike/cargo-serial.ps1 test --locked --lib --features "sdk-js sdk-material" --target x86_64-pc-windows-msvc sdk_spike
   ```

3. 一张与设计稿同尺寸的截图（插件任务必附）。
4. 没做完或做法与说明不同的地方，逐条写明原因。

## 共用约定

- 坐标单位是岛的逻辑像素。展开面板宽 419，内容区宽 387（左右各留 16），原点在内容区左上角，标题行以下。
- 颜色可以写 `#RRGGBB`、`#RRGGBBAA`，也可以写主题令牌：`$accent`（插件强调色）、`$text`、`$muted`、`$dim`、`$card`、`$track`、`$bg`。换皮肤时令牌自动跟随。
- 时间统一用 Unix 秒。时区用 IANA 名称，例如 `Asia/Shanghai`。
- 文档写进 `docs/plugins/sdk-v1-design.md` 的新小节「自绘画布」，字段写进 `docs/plugins/sdk-v1.schema.json`，并在 `spike/examples/` 各加一个样例。
