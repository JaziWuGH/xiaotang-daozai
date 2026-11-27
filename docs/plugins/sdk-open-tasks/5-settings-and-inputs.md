# 第 5 步：设置类型、时间输入、数据文件与调度

这一步补齐脚本插件最常用的四类基础能力：

- 更多设置类型；
- 当前时间和时区输入；
- 包内只读数据文件；
- 按时间点调度刷新。

另外把私有存储从输出里拆出来，单独放宽上限。分支为 `feat/plugin-settings-inputs`，共同规则见 [README](README.md)。

## 先读

- `spike/local.rs`：`Field`、`valid_field`、`settings_values`、`render_input`、`tick_features`，以及 `Package::read` 读包的部分。
- `spike/local_ui.rs`：插件设置的绘制，入口在 `settings()`。
- `src/extras.rs`：宿主的设置控件，包括开关、分段、数字和文本行。
- `docs/plugins/sdk-v1-design.md` 的设置说明，其中那一行写着「脚本插件的开关、分段、数字 · 待宿主实现」。

## 要做的

### 设置类型

以下类型都复用宿主现有控件，放在所属设置页，标题旁带「插件」小标。`default` 一律用字符串保存，保持和现有格式一致。

| type | 额外字段 | 保存格式 | 控件 |
| --- | --- | --- | --- |
| `switch` | 无 | `"true"` 或 `"false"` | 紫色开关 |
| `choice` | `choices:[{value,label}]`，2 到 6 项 | 选中的 value | 分段（不超过 4 项）或下拉 |
| `number` | `min`、`max`、`step`、`unit?`、`precision?` | 数字字符串 | 数字输入加单位 |
| `time` | 无 | `"HH:MM"`，24 小时制 | 时间输入 |
| `weekdays` | 无 | 由 `1` 到 `7` 组成、用逗号分隔，比如 `"1,2,3,4,5"` | 7 个可选小圆 |
| `list` | `item:[...]`，内含 1 到 3 个子字段（只能是 text、choice、number、date、time），`maxItems` 不超过 8 | JSON 数组字符串 | 可增删排序的行 |

补充说明：

- 材质插件的 `number` 目前单独校验。改完后两边共用一套校验，但材质原有的限制不能放宽。
- `list` 用于世界时钟的城市列表、多个倒计时、多只自选品种这类需求，这样用户不用把同一个插件装好几份。

### 时间与时区输入

脚本 input 新增以下字段：

- `now`：Unix 毫秒时间戳。
- `tz`：`{offset_minutes, name}`，表示本机时区。
- `zones`：清单声明了 `timezones:true` 时才提供。宿主为设置里出现的 IANA 时区名，算出当前的 `offset_minutes`，并标出是否处于夏令时。Rust 侧可以用 `chrono-tz`，但先检查 `vendor/` 里有没有；没有的话写清楚引入它的影响，或者内置一张常用城市表。

原有的 `today` 保留不动。

### 包内只读数据文件

- 清单新增 `data:["holidays-2026.json", ...]`。
- 每个文件必须是根目录下的 UTF-8 JSON，单个不超过 64 KiB，合计不超过 256 KiB。
- 安装时连同快照一起保存，脚本通过 `input.data["holidays-2026.json"]` 拿到解析好的对象。
- 不能包含链接或越出包目录的路径。打包命令 `--plugin-pack` 和测试集市 ZIP 都要带上这些文件。

### 按时间点调度

- 输出可以带 `next_refresh_at`，单位是 Unix 秒。
- 宿主在这个时间点和正常周期之间取较早的那个，作为下一次调用的时间，但离现在至少 60 秒。
- 用途：实时工资在上班、下班时刷新；倒计时在零点刷新；节假日插件在假期开始时刷新。

### 私有存储放宽

- `storage` 从 16 KiB 的输出上限里拆出来，单独限制为 256 KiB。
- 写满时返回明确的错误，并在插件详情里提示。

### 样例

- 在 `spike/examples/` 加「通用倒计时」，用到 `choice`、`switch`、`date`、`next_refresh_at`、待办卡片和 `calendar.marker`。
- 再加「节假日和调休」，自带 2026 年数据文件，用到 `data`、`calendar.marker`、待办卡片。
- 2027 年的数据由作者每年发新版本更新，在文档里写明。

## 验收标准

- 单元测试至少覆盖：
  - 每种设置类型的合法值、非法值和默认值；
  - `list` 的上限；
  - 旧插件包的设置照常读取；
  - `now` 和 `tz` 的输入；
  - `zones` 的偏移（拿一个有夏令时的地区测）；
  - 数据文件的大小限制、路径限制和打包；
  - `next_refresh_at` 的取值（太近、太远、已经过去）；
  - storage 的新上限。
- egui 界面测试：每种新设置控件都能改值并保存，标题旁有「插件」小标。
- 跑下面的命令，全部通过：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File spike/cargo-serial.ps1 test --locked --lib --features "sdk-js sdk-material" --target x86_64-pc-windows-msvc sdk_spike
```
