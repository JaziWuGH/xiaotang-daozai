# 第 3 步：状态页组件、实时组件与主题令牌

本步扩充状态页组件，并加入由岛仔逐帧计算的实时组件、敏感值隐藏和受控的主题令牌。分支为 `feat/plugin-status-components`，共同规则见 [README](README.md)。

## 先读

- `src/plugin_status.rs`：`PluginStatus`、`StatusLayout`、`StatusComponent`、`StatusStyle`。
  - 已有组件：state、summary、progress（环或条）、metrics、alert、updated_at。
  - 已有样式：accent、density、surface。
- `src/plugin_status_ui.rs`：绘制。
- `spike/status_source_tests.rs`。
- `docs/plugins/sdk-v1-design.md` 的状态页说明。

## 要做的

### 普通组件

每种组件都要做三件事：在 `PluginStatus` 里加一个数据字段，在 `StatusComponent` 里加一种类型，在 `valid()` 里设上限。

- **`list`**：`items` 最多 8 项，每项为 `{label, detail?, state?, action?}`。`state` 复用 `StatusState`，用颜色点表示。`action` 归第 1 步实现，这里只要能解析并保留下来。
- **`chart`**：`series` 为 2 到 48 个有限数字，画成迷你折线。宿主负责画面积填充、淡网格和末点强调。可带 `series_label`（最多 12 字）。
- **`text`**：`note`，最多 300 字的多行说明。
- **`badge`**：`tags` 最多 4 个短标签，每个最多 8 字。

### 实时组件

实时组件由岛仔在本地逐帧计算，脚本不用每秒运行。脚本只返回下面的参数，宿主按当前时间算出要显示的值。插件停用或页面不可见时停止重绘。

- **`counter`**：跳动的数字。参数为 `{start_value, per_second, start_at, until?, decimals(0–4), prefix?, suffix?}`。显示值为 `start_value + per_second × (now − start_at)`，超过 `until` 后停在终值。实时工资就用它。
- **`clock`**：某个时区的当前时间。参数为 `{label, utc_offset_minutes, format:"HH:mm" | "HH:mm:ss"}`。时区偏移由第 5 步的宿主时区换算提供，脚本也可以自己算。世界时钟用它。
- **`countdown`**：倒数到某个时间点。参数为 `{label, target_at, format:"d天 HH:mm" | "HH:mm:ss"}`，到点后显示 `done_label`。距离下班、倒计时都用它。

三种实时组件各自最多出现 4 个，这样世界时钟可以列 4 个城市。

刷新频率：显示到秒的每秒重绘一次，显示到分钟的每分钟重绘一次。不能为了它们让整个岛每帧重绘，要用 `request_repaint_after` 精确安排下一次刷新。

### 敏感值

- `metrics`、`counter` 和 `list` 的项可以标 `sensitive: true`。
- 宿主默认把敏感值显示为打码样式，用户点一下才露出，30 秒后自动再盖上。
- 系统锁屏、录屏或截图排除开启时，敏感值始终打码。截图排除功能本体已有，见 `src/native_capture.rs`。
- 实时工资的金额必须标成敏感。

### 主题令牌

- `StatusStyle` 新增 `emphasis`，取值 `normal` 或 `strong`。
- `StatusStyle` 新增 `icon`，从宿主内置图标里挑一小组名字开放，例如 printer、battery、cloud、cup、clock、chart、coin、calendar。宿主没有对应图标的名字就不开放。
- 绘制前，按当前皮肤的卡片底色自动校正 accent，保证对比度至少 3:1。
- 字体、圆角、间距、动效仍由宿主决定，不开放给插件。

### 布局与密度

- 布局上限从 6 个组件放宽到 10 个。实时组件按出现次数计入，其他组件仍然每种只能出现一次。
- 紧凑密度下，由宿主决定收起哪些组件，或只显示前几项。

### 不要做的

按钮和输入框不做，它们属于第 1 步，那边会在同一个枚举里加 `actions` 和 `fields`。这边的改动保持枚举容易合并。

### 样例与文档

- 在 `spike/examples/` 加一个「实时工资」样例，封面可以复制 payday 的。
  - 第 5 步的设置类型还没合进来时，先用文本设置项代替，并在清单里注释说明。
  - 要用到 `counter`、`countdown` 和敏感值。
- 在 `spike/examples/` 再加一个「世界时钟」样例，用 4 个 `clock`。
- 更新 `docs/plugins/sdk-v1.schema.json` 的 `statusLayout` 部分。

## 验收标准

**单元测试**至少覆盖：

- 新字段的上限和非法值；
- 布局上限；
- 实时组件在 `until` 和 `target_at` 前后的取值；
- 跨午夜的时区换算；
- 敏感值默认打码；
- accent 对比度校正。

**egui 绘制测试**：同一份状态在以下条件下渲染，文字不越出卡片，也不 panic。

- 不同皮肤，深浅两种主题；
- 标准和紧凑两种密度；
- 100% 和 200% 缩放。

另外要确认：只有秒级实时组件时，重绘间隔约为 1 秒，不是每帧都重绘。

**兼容性**：旧的 statusLayout 插件包不用改就能通过。

**测试命令**：下面这条全部通过，再把过滤词换成 `sdk_spike` 跑一遍，也全部通过。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File spike/cargo-serial.ps1 test --locked --lib --features "sdk-js sdk-material" --target x86_64-pc-windows-msvc plugin_status
```
