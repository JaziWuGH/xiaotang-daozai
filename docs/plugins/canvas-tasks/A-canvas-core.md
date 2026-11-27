# A：画布核心

分支 `feat/plugin-canvas`，从本地 `main` 开出。共同规则见 [README](README.md)。

本任务交付「能画出来」：解析和校验画布、静态绘制、主题令牌、包内图片、和标准组件混排。时间绑定、动画、点按由 B 做，但数据结构要在这里一次定好，B 只补求值和行为。

## 先读

- `src/plugin_status.rs`：`PluginStatus`、`StatusLayout`、`StatusComponent` 的解析和上限。
- `src/plugin_status_ui.rs`：状态页如何按布局绘制，`draw_layout`、`component_height`。
- `src/plugin_status_components.rs`、`src/plugin_status_realtime.rs`：现有组件和实时组件。
- `spike/local.rs`：`Output` 解析、`render_input`、输出大小上限。
- `spike/capabilities.rs`：能力表。
- `spike/appearance_assets.rs`：资源包里 SVG 和 PNG 的读取与校验，图片部分可以复用。

## 清单和输出

- 新能力 `status.canvas`，运行环境 js，作为输出加进能力表，要求同时声明 `status.page`。
- `refresh`、`action`、`event` 的返回值新增字段 `canvas`，与 `status` 同级。
- `statusLayout.components` 新增类型 `{"type":"canvas"}`。它在哪个位置出现，画布就画在哪里；如果布局里只有它，画布占满整页内容区。
- 画布的整体结构如下：

```json
{
  "height": 196,
  "nodes": [ … ],
  "binds": { … }
}
```

- `height`：40 到 360，决定内容区高度，岛的展开高度随之变化。
- `binds`：本任务只解析并保存，不求值，详见 B。
- 含画布的输出上限从 16 KiB 放宽到 64 KiB，其余输出仍是 16 KiB。

## 图元

每个节点是一个对象，键名决定类型。

| 类型 | 写法 | 说明 |
| --- | --- | --- |
| 矩形 | `{"rect":[x,y,w,h], "r":8}` | `r` 可以是数字，也可以是 `[左上,右上,右下,左下]` |
| 圆 | `{"circle":[cx,cy,r]}` | |
| 弧 | `{"arc":{"c":[x,y],"r":40,"start":-90,"sweep":210}}` | 角度单位为度，0 度朝右，顺时针为正 |
| 直线 | `{"line":[x1,y1,x2,y2]}` | |
| 折线 | `{"points":[[x,y],…], "area":118}` | `area` 给出时，填充到这条水平线，用于面积图 |
| 路径 | `{"path":"M0 0 L10 10 Q… C… Z"}` | 只支持 M、L、H、V、Q、C、Z，大小写都要支持 |
| 文字 | `{"text":"距 元旦","at":[x,y],"size":11,"weight":700,"align":"left"}` | `at` 是基线位置；可选 `max_width`、`lines`（最多几行）、`font`（见 C）|
| 图片 | `{"image":"cover.png","rect":[x,y,w,h],"fit":"cover"}` | 只能引用包内 PNG 或 SVG，`fit` 可选 `cover`、`contain` |
| 分组 | `{"group":[…], "clip":[x,y,w,h], "translate":[x,y], "rotate":15, "scale":1.2}` | 变换作用于整组 |

通用样式字段：

- `fill`、`stroke`：颜色或渐变。渐变写成 `{"linear":[[x1,y1],[x2,y2]],"stops":[[0,"#FFE58A"],[1,"#00000000"]]}`，`radial` 写法同理。
- `width`：线宽。`cap` 可选 `round` 或 `butt`。`dash`：`[实线长, 间隔长]`。
- `opacity`：0 到 1。
- `shadow`：`sm`、`md`、`lg` 三档，由宿主统一绘制。
- `glow`：`true` 或颜色，宿主用同色模糊外发光绘制，参考性能页的发光圆环。
- `id`：可选。B 做动画和点按时用它对应前后两次输出里的同一个节点。

## 绘制

- 用 egui 的 `Painter` 和 epaint 实现。路径和贝塞尔曲线走 epaint 的曲线类型；渐变用顶点着色的网格；文字走 galley，字号和字重自由。
- 坐标按缩放比例换算。100% 到 200% 缩放下线条不能发虚，文字不能越出 `max_width`。
- 主题令牌按当前皮肤取色，深浅主题都要正确。
- 图片在安装时读入并缓存成纹理。每个包的图片合计不超过 512 KiB，单张不超过 1024×1024。
- 静态节点按「画布内容 + 缩放比例」缓存细分结果，内容不变时不重新计算。

## 校验与上限

宿主拒收不合规的画布，在开发调试面板写明原因，页面显示「插件画面有误」占位，不能崩溃。

- 节点总数（含分组内）最多 1500，嵌套最多 8 层。
- 路径命令合计最多 4000 个，文字合计最多 2000 字。
- 所有数字必须是有限值，坐标在 -2000 到 2000 之间，字号在 6 到 96 之间。
- 只能引用包内存在的图片。路径里出现 `..` 或绝对路径直接拒收。
- 单帧绘制超过 4 毫秒时，改用上一帧的缓存图像，并在开发调试面板提示作者。

## 样例与文档

- 在 `spike/examples/canvas-demo/` 加一个演示插件，用到上面所有图元、渐变、阴影、发光和一张包内图片。
- 在 `sdk-v1-design.md` 新增「自绘画布」小节，在 `sdk-v1.schema.json` 补上字段定义。

## 验收标准

- 单元测试覆盖：
  - 每种图元的解析；
  - 每条上限和非法值的拒收；
  - 主题令牌取色；
  - 图片路径越界拒收；
  - 混排布局的高度计算。
- egui 绘制测试：演示插件在两种以上皮肤、深浅主题、100% 和 200% 缩放下渲染，不 panic，文字不越界。
- 现有 statusLayout 插件不用改就能照常显示。
- 静止的画布页不产生持续重绘。
