# 第 6 步：岛上常驻位与开发调试

本步有两部分：

- **岛上常驻位**：岛收起时，也能显示插件提供的一小段内容，比如今日工资、金价、下个假期还有几天。
- **开发调试**：降低作者自己动手写插件的门槛，减少他们为"能不能做"来问。

分支用 `feat/plugin-pinned-devtools`，共同规则见 [README](README.md)。

## 先读

- `src/app.rs`：岛收起时胶囊里显示什么、怎么排优先级。可以搜现有的常驻信息，例如音乐、未读数、额度。
- `spike/local.rs`：`tick_features`、`cards`。
- `spike/plugin_author_ui.rs`：开发者工具。
- `spike/local.rs` 里 `--plugin-test`、`--plugin-pack` 的实现。
- `docs/plugins/sdk-v1-design.md` 中"常驻、固定、临时卡 · 待宿主实现"那一行。

## 要做的

### 岛上常驻位（能力 `island.pinned`）

**插件怎么提供内容**

- 输出里带 `pinned:{text, value?, icon?}`，`text` 最多 12 个字。
- `value` 可以引用第 3 步的 `counter` 或 `countdown` 参数，由宿主在本地跳动显示；也可以标 `sensitive: true`。

**用户怎么选**

- 用户在插件设置里打开「在岛上常驻显示」，默认关闭。
- 同一时间最多 3 个插件常驻，按用户排定的顺序轮流显示，在岛的设置里调整顺序。
- 超过 3 个时提示用户替换哪一个。

**和本体内容怎么排**

- 本体正在显示的高优先级内容时插件不出现，包括来电、通知弹窗、音乐歌词。
- 具体优先级参考现有规则，写进文档。

**怎么画**

- 先用现有胶囊文字样式画出能用的版本。最终视觉由验收方来做。
- 实时值的重绘频率同第 3 步：秒级每秒重绘一次，不要每帧重绘。

**启停**

- 插件停用、所属模块关闭、或用户关掉常驻时，立即撤下。

### 开发调试

- **保存后自动重载**：开发者工具里勾选「监视文件变化」后，从本地清单安装的插件在 `plugin.json`、入口脚本或数据文件保存后 1 秒内重新安装并刷新。旧数据保留。只对本地开发包生效，集市安装的不受影响。
- **脚本日志**：脚本里可以用 `console.log`，每次调用最多 20 行、每行 200 字。在开发者工具里查看最近 100 行，生产环境不写磁盘。
- **模拟输入**：`--plugin-test plugin.json --input mock.json` 用指定的 settings、storage、now 和 sources 跑一次 refresh，打印输出和用时，方便作者不联网也能试。
- **错误定位**：脚本报错时显示行号和列号。
- **能力自检**：`--plugin-test` 输出这个清单用到的能力，以及每项在当前宿主上是"可用""实验"还是"未支持"。
- **教程**：在 `spike/plugin_guide_ui.rs` 的教程里写明这些工具怎么用。

## 验收标准

**单元测试**至少覆盖：

- 常驻位最多 3 个；
- 停用后撤下；
- 高优先级内容出现时让位；
- 敏感值打码；
- 文件变化触发重载，以及防抖；
- 日志条数和长度上限；
- `--input` 模拟输入；
- 报错带行号；
- 能力自检的输出。

**egui 界面测试**：打开常驻后，岛收起时能看到插件内容。

**测试命令**：下面这条全部通过。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File spike/cargo-serial.ps1 test --locked --lib --features "sdk-js sdk-material" --target x86_64-pc-windows-msvc sdk_spike
```
