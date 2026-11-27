# P0：插件仓投稿校验支持新插件

插件仓分支 `feat/submission-status-canvas`。无依赖，第一周就做。共同规则见 [README](README.md)。

## 背景

`scripts/submission.py` 目前接受三类插件：

- 待办、日程归属的脚本插件；
- 快捷切换资源包；
- 无线兼容资源包。

五个官方插件都用到状态页、画布、常驻和联网，现在的校验会全部拒收。

## 要做的

- 接受 `owner: "status"` 的脚本插件。
- 接受以下能力：
  - `status.page`、`status.canvas`
  - `island.pinned`
  - `action`、`event`、`notice`
  - `timer`、`storage`、`settings`
  - `card`、`calendar.marker`
- 组合规则与岛仔的 `spike/capabilities.rs` 保持一致：
  - `status.canvas` 和 `action` 都要求同时声明 `status.page`；
  - 归属必须带上自己的输出。
- 接受清单的以下字段，并做基本类型检查：
  - `statusLayout`、`network`、`data`、`timezones`、`minRefreshSeconds`、`attribution`、`events`。
- `network.hosts` 必须是完整域名，`sources` 的 URL 必须是 https，且主机在 `hosts` 里。
- 包内允许出现的文件：
  - `plugin.json`、`main.js`、`cover.png`；
  - 清单 `data` 声明的 JSON（单个 64 KiB，合计 256 KiB）；
  - 画布引用的 PNG 和 SVG（合计 512 KiB）；
  - `LICENSE`、`README.md`；
  - `fixtures/` 和 `TESTING.md`。这两项只用于开发，打包时排除。
- 字体包 `appearance.font`：
  - 允许 TTF、OTF，单个 8 MiB，整包 16 MiB；
  - 必须带 `LICENSE` 或 `OFL.txt`。
- 在测试文件里为每条新规则写通过和拒收两类用例。原有 18 项测试保持通过。

## 验收

- `python -m pytest -q scripts` 全部通过。
- 用 `spike/examples/gold-price`、`live-pay`、`world-clock`、`sticky-notes`、`countdown` 的清单做输入，`read_plugin` 都能通过。
