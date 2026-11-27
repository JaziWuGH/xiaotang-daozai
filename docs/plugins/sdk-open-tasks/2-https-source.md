# 第 2 步：联网取数与凭据

开放受控联网，一个插件最多可以有 3 个来源。密钥由岛仔托管，刷新间隔允许用户调整，集市加版本撤回标记。分支为 `feat/plugin-https-source`，共同规则见 [README](README.md)。

## 先读

- `spike/source_read.rs`：现有的文件和本机 HTTP 只读来源。
- `spike/local.rs`：`source_kind`、`set_source_grant`，以及 `tick_features` 里的后台读取。
- `spike/local_ui.rs`：`source_dialog` 授权弹窗。
- `src/mail.rs` 的 `mod secrets`：已经在用 `keyring` 写 Windows 凭据管理器。
- `spike/market.rs`、`spike/market_state.rs`。
- `docs/plugins/sdk-v1-design.md` 的第 5 节和第 12 节。

## 要做的

### 清单写法

权限写成 `"permissions":{"source.read":"https"}`。

清单新增 `network` 字段：

```json
"network": {
  "hosts": ["xaus.com", "api.example.com"],
  "sources": [
    {"key": "gold", "url": "https://xaus.com/api/spot?currency={settings.currency}"},
    {"key": "fx", "url": "https://api.example.com/rate?base=USD"}
  ],
  "credential": {"label": "API Key", "header": "x-access-token", "prefix": ""},
  "minRefreshSeconds": 60
}
```

字段规则：

- `hosts`：1 到 4 个完整域名。不接受通配符、IP、端口，也不接受 localhost。
- `sources`：1 到 3 个来源，`key` 在同一插件里不能重复。`url` 里可以写 `{settings.<key>}` 占位符，宿主做 URL 编码后填进去。填完后的地址必须满足四条：协议是 https；主机在 `hosts` 里；不含账号密码段；端口是 443 或不写。占位符只能出现在路径和查询串里，不能出现在主机部分。
- `credential`：可选。声明后，宿主在每个来源的请求里注入同一个请求头。
- `minRefreshSeconds`：可选，范围 60 到 86400，默认等于 `refreshSeconds`。

### 授权

- 新增 `SourceKind::Https`。授权弹窗列出全部来源的主机，用户确认一次就覆盖所有来源。
- 声明了 `credential` 的插件，弹窗里多一个密钥输入框，内容遮挡显示，标题用 `credential.label`。
- 密钥存进 Windows 凭据管理器，复用 `keyring`。服务名单独起一个，键用插件 ID。
- 记录 JSON 和日志里绝不能出现密钥。
- 撤销授权或卸载插件时，同时删除密钥。
- 插件更新后，如果 `hosts` 或 `sources` 变了，原授权失效，用户要重新授权。

### 读取

- 只发 GET，关闭环境代理。
- `redirects(0)`，收到 3xx 一律算失败。
- 用自定义 resolver。解析出地址后，丢弃回环、私网、链路本地、CGNAT、ULA、组播和未指定地址，剩下为空就失败。
- 每个来源超时 10 秒，响应最多 256 KiB，必须是 UTF-8。
- 宿主按 `header` 加 `prefix` 注入密钥，脚本拿不到密钥。
- 每次取数都重新检查地址规则。
- 脚本拿到 `sources:{gold:{status,text}, fx:{status,text}}`。旧的单一 `source` 输入对本地文件和本机来源保持不变。
- 连续失败时退避重试，间隔依次是 1、2、4、8 倍，上限 30 分钟。成功一次就恢复正常间隔。

### 刷新间隔

清单声明了 `minRefreshSeconds` 时，插件设置页自动多出一项「刷新间隔」。用户可以在 `minRefreshSeconds` 到 86400 秒之间调整，用来照顾免费接口的额度。

### 数据来源标注

- 清单新增 `attribution:{label, url?}`。`url` 的主机必须在 `hosts` 里。
- 宿主在状态页和插件详情里显示「数据来源 xxx」以及最近一次成功取数的时间。具体怎么画由第 3 步和验收方定，你这边负责把数据提供出来。

### 版本撤回

- 集市目录的条目可以带 `revoked:["1.0.1", ...]`。
- 已安装的版本被撤回时：停用该插件，在「已安装」里用警示色标出；如果有更高的未撤回版本，提供更新。
- 被撤回的版本不能安装。
- 补上 `spike/market.rs` 的解析和对应测试。

### 样例

- 在 `spike/examples/` 加一个「金价」样例：
  - 主来源 xaus.com，需要的话再加一个汇率来源；
  - storage 保留最近 48 个价格点，供第 3 步的迷你折线使用；
  - 用户可以设一个提醒价位，涨过或跌破时用第 1 步的 `notice` 提醒；
  - 显示数据来源；
  - 不给任何投资建议。
- 另写一份带 `credential` 的清单示例放进文档，以 GoldAPI.io 为例。

## 验收标准

- 单元测试至少覆盖下面几项，且不依赖外网，用本地假服务或直接测函数：
  - 地址校验：非 https、IP、`hosts` 之外的主机、带账号密码、非 443 端口、占位符出现在主机部分；
  - 占位符的 URL 编码；
  - resolver 过滤私网和回环地址；
  - 拒绝跳转；
  - 超大响应；
  - 多来源里一个失败时其他来源照常；
  - 退避重试；
  - 刷新间隔的上下限；
  - 密钥不出现在记录 JSON 里；
  - 撤销后停止读取；
  - 撤回版本被停用、禁止安装。
- 下面这条命令全部通过：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File spike/cargo-serial.ps1 test --locked --lib --features "sdk-js sdk-material" --target x86_64-pc-windows-msvc sdk_spike
```
