# 本机通知来源接口 v1

本机接入端可向岛仔发送短时通知、更新常驻状态，以及接收和确认用户点击发出的操作。岛仔负责呈现提醒、未读标记和状态详情。接入端自行运行，负责取数和执行操作，例如打开原会话。

## 用户授权

1. 在「设置 → 消息提醒 → 本机通知来源」填写接入端提供的来源 ID，以及要显示在岛上的名称。
2. 点「授权来源」，再点「复制凭据路径」，把路径交给接入端。凭据文件内含访问令牌，只交给自己信任的接入端。
3. 开启提醒总开关。要在岛上阅读接入端提供的标题和正文，再开启「显示通知正文」。

撤销授权会清除该来源的通知、状态和待处理操作，并移除凭据文件。没有已授权来源时，岛仔关闭本机接口监听。

来源 ID 建议使用 `作者.接入端`，例如 `example.agent`，最长 64 个 ASCII 字符。各段允许小写字母、数字和连字符，并以字母或数字开头。每个接入端只读取自己的凭据文件，不把凭据内容写入日志或上报。默认路径是 `%APPDATA%\Daozai\plugin-notices\<来源 ID>.json`。设置中复制的路径也适用于自定义数据目录。

## 连接

凭据文件包含 `version`、`host`、`port`、`source`、`token`。当前版本为 `1`。每次连接岛仔前重新读取此文件，避免使用上次启动的端口和令牌。

通过 TCP 连接凭据中的 `127.0.0.1` 和端口。每个连接发一行 UTF-8 JSON，以换行结束。岛仔回一行 JSON 后关闭连接。每个请求均需带凭据里的 `version`、`source`、`token`，以及一个 `method`。成功响应含 `ok: true`，失败响应含 `ok: false` 和可用于诊断的 `error`。

| method | 附加字段 | 行为 |
| --- | --- | --- |
| `status` | 无 | 返回 `preview`。为 `false` 时，接入端可省略可选的 `markdown`。 |
| `notify` | `id`、`title`，可选 `markdown`、`session`、`action_label` | 发送短时通知。弹窗收起后可从岛上的未读角标进入。 |
| `status_update` | 与 `notify` 相同，外加 `ttl_seconds` | 更新该来源唯一的状态卡以及有效期。用户可从岛上的状态角标打开详情。更新状态时不弹通知。 |
| `status_clear` | `id` | 撤下 ID 匹配的当前状态和待处理动作。重复撤下或 ID 已变更时也返回成功。 |
| `complete` | `id`，可选 `session` 和 `markdown` | 兼容旧版的任务完成通知。 |
| `poll_open` | 无 | 取本来源的一次待处理操作。返回 `open: null` 或 `open: { "request_id": "...", "session": "..." }`。 |
| `ack_open` | `request_id`、`opened` | 操作结束后回执。确认成功才回 `opened: true`，例如确实已打开指定会话。失败或无法确认时回 `false`。 |

`notify`、`status_update`、`status_clear` 需要包含这些方法的验收版。协议版本仍为 `1`，旧接入端可继续使用 `complete`。

## 通知和状态

将示例中的 `<token>` 换成凭据文件中的令牌。每行是一个独立请求。

```json
{"version":1,"source":"example.agent","token":"<token>","method":"notify","id":"notice-42","title":"导出完成","markdown":"可以打开原任务查看结果。","session":"task-42","action_label":"查看任务"}
{"version":1,"source":"example.agent","token":"<token>","method":"status_update","id":"job-7","title":"处理中 · 37%","session":"task-7","action_label":"查看进度","ttl_seconds":8}
{"version":1,"source":"example.agent","token":"<token>","method":"status_clear","id":"job-7"}
```

`id`、`title`、`session`、`markdown` 分别最多 128 字节、120 字节、512 字节、32 KiB，均按 UTF-8 计算。`action_label` 最多 6 个字符且不超过 24 字节，提供按钮文案时需同时提供 `session`。整条请求最多 128 KiB。

- 同一通知重试时沿用原 `id`，不同通知使用新 ID。宿主按来源在有限范围内去重。每个来源每秒最多发送 4 条新通知，队列满或触及频率限制时返回 `busy`，接入端应退避后重试。
- `ttl_seconds` 取 2 至 120 的整数。接入端刷新同一个状态时沿用 `id`，操作目标变化时换新 ID。每个来源的状态更新间隔至少 250 毫秒，过早更新会返回 `busy`。来源停止更新后，岛仔在有效期结束时撤下卡片及待处理动作。

## 操作回执

`session` 是接入端自己定义的操作标识。提供它可让用户打开目标会话或触发接入端支持的其他动作。岛仔只把标识交回所授权的来源。接入端运行期间可定期调用 `poll_open`。收到请求后自行执行，在请求发出后 5 秒内提交 `ack_open`。超时或失败时，用户可从仍有效的角标重试。打开通知详情或成功完成通知动作后，岛仔会清除其未读标记。

## 隐私和启停

关闭正文预览会隐藏接入端提供的标题、正文和按钮文案。锁屏时隐藏详情。内容和操作标识不会写入通知历史，正文以文本渲染，支持常见的 Markdown 标题、列表和代码块，预览最多显示 128 行。

关闭提醒总开关会清除当前通知、状态和操作队列。新写请求返回 `disabled`，`poll_open` 返回空结果，旧 `complete` 按兼容约定返回成功并丢弃该提醒。重新开启后，接入端可发送新的状态或通知。

## 本机示例

将 `example.agent` 授权为「接口示例」，然后把设置中复制的凭据路径传给下面的 Python 3 脚本。脚本只依赖 Python 标准库。它会发出一条通知，更新一张模拟进度卡，并在您点「打印回执」后在控制台显示回执。

```powershell
python .\docs\plugins\examples\notice-provider.py "凭据文件路径"
```

[源代码](examples/notice-provider.py)
