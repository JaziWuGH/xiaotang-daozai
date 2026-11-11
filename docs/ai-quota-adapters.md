# AI 额度适配（开发中，未发布）

实现基线：`main` `39413b3c65d0c91d598507bc6d1ab3be0c6d1105`，2026-11-08 核实。

## 入口与数据含义

- OpenCode Go：套餐 API Key，固定官方 usage 地址。显示官方滚动／周／月使用率，不冒充 Zen 按量余额。未证实的周期长度不推断。
- MiniMax：中国区 / 国际区必须手选，分别使用 `api.minimax.cn` / `api.minimax.io`。查询 Coding Plan，优先官方当前 `general` 额度池，兼容 `MiniMax-M*` 旧池；不会相加共享池。剩余百分比、旧 remaining count 语义、周倍率独立处理。
- GLM：中国区 `open.bigmodel.cn` / 国际区 `api.z.ai`，查询 Coding Plan，不查询推理 API 钱包。使用官方脚本的原始 Authorization Key 格式。未验证的单位编码和重置时间不转换；工具额度不能单独伪装为主套餐额度。「获取密钥」按地区打开官方套餐入口，提示使用套餐 Key；按 HTTP 和业务错误区分认证、权限、套餐状态及限流，不回显服务端原文，不跨地区重试。
- New API：选择协议，输入 HTTPS 站点地址 + API Key。`/api/usage/token/` 返回的数值标为「站点配额」，不擅自换算美元。
- One API：独立读取 `/dashboard/billing/subscription` 和 `/dashboard/billing/usage`。站点可配置展示单位，即使字段名含 USD 也不标美元。官方无限额哨兵与真实数值存在歧义时显示未知，不画假进度。
- Sub2API：独立 `/v1/usage`，区分 Key 限额、订阅周期和钱包；保留协议声明的 USD、周期和显式重置。到期时间不是重置时间。
- Kimi：保留既有 Kimi Code 行为，没有扩大范围。

中转地址要求站点根地址，原样保留反向代理部署前缀（包括路径中的 `/v1`），不会猜测或删除 API 后缀。拒绝 HTTP（含 localhost）、用户名/密码、查询参数和 fragment。不猜测协议或向另一个厂商/地区自动重试。

## 安全与性能

- 岛仔只调用指定额度查询接口，但不会缩减 Key 本身已有的权限；不要把普通 API Key 宣传为只读 Key。
- Key 只进入 Windows 凭据管理器 `Daozai.ApiQuota`。配置和缓存只保存随机凭据引用、协议与地址/地区；日志及错误不回显响应正文或 Key。
- 验证异步执行且最多一项在途；取消后仍等待当前有界请求结束，不累计线程。取消阻止后续 One API 请求和 Key 保存。切换服务清空输入中的 Key。连接/移除先持久化配置，再清理旧凭据。清理日志仅存随机引用，写入失败不继续修改；清理失败保留引用供重试。启动清理必须严格确认已保存配置可读、结构有效，不把配置损坏视为空账号。
- 共享既有代理设置，15 秒请求超时，禁用重定向，解码后响应上限 1 MiB。One API 两次顺序请求总耗时可能约 30 秒。
- 每个新增协议有独立单线程队列；最新刷新覆盖尚未开始的旧请求，不累积无界线程，不等待其他协议完成。原有浏览器类服务维持同组串行。
- 429 / 临时服务错误按独立账号退避，最长退避 1 小时。重连、修改地址和地区后旧请求结果按账号、协议、凭据引用、来源校验丢弃。
- 刷新失败保留上次数据并显示错误/陈旧提示。重启缓存按协议、凭据及地址/地区绑定；旧缓存缺少新增字段仍能读入。
- 未知、无限、零额度分开处理；不把缺失字段当 0。新增图标是自绘通用 Key 标识，不冒充官方品牌素材。

## 暂不启用

- Qoder：官方国区 `@qodercn-ai/qodercn-agent-sdk` 与国际 SDK 各有 `noPrompt` / `getUsageInfo`。尚未在隔离运行时验证登录复用、无额外消费、取消及 Windows 兼容性；不自动安装，不解密私有凭据。
- 火山方舟：Agent Plan `GetAFPUsage` 需要 AK/SK 签名，不等于推理 Key；Coding Plan 官方 `arkcli usage plan --product coding-plan --format json` 的 SSO、响应契约及权限待验证。不隐式扩大权限，不把两产品混为一谈。

## 验证状态

已进行：官方源代码协议核对、两轮模块交叉静态审查、`git diff --check`、Linux library 编译。用户随后授权安装官方 Rust stable 1.99.1，仅安装在独立云工作区，项目依赖严格使用 Cargo.lock；未安装系统包。

新增 34 个合成测试全部通过，覆盖套餐解析、金额/单位、未知和无限、响应错误、地址安全、缓存来源绑定、旧缓存兼容、队列合并及晚到结果过滤。

原始实现的 Linux `cargo build --locked --bin daozai` 被 main.rs 两处既有 Windows-only `system_state` 引用阻挡；`--lib` 测试被既有 WorkBuddy 手工登录测试引用 Windows-only login 模块阻挡。为继续验证，另建一次性副本，仅为以上三处增加平台条件；这些平台兼容调整没有加入本补丁。

最终修正版在该验证副本执行 `cargo test --locked --all-targets --features record --no-fail-fast -- --test-threads=1`，其中 library 测试：663 通过、4 失败、58 忽略。四个失败已在干净 main 的独立基线副本复现，位置和断言值一致：
- 日历日期横向留白（ui_tests.rs:3469）
- 邮件报告保存/失败路径（ui_tests.rs:4556）
- 既有单额度环 100% 的文字边界（ui_tests.rs:1756）
- 设置主题时段文字垂直居中（ui_tests.rs:4430）

随后单独补跑二进制与全部 Rust 集成测试目标：另 10 项通过，0 失败；全部 example 测试目标运行成功（各含 0 项测试）。`calendar_credentials` 和 `performance` 在 Linux 含 0 项可运行测试，不代表 Windows 专项通过。总体不是全绿，不把已复现的基线失败隐藏或算作通过。

**尚未验证**：Windows 编译与 Credential Manager 实测、Windows 原生 GUI、真实账号请求、安装包及发布流程。无账号凭据使用，无推送、发布或用户电脑修改。

Windows 隔离环境仍需执行仓库要求：

```
cargo build --locked --bin daozai
cargo test --locked --all-targets --features record -- --test-threads=1
```

Windows 验收另用临时 `DAOZAI_DATA_DIR`；不运行 ignore 的私人账号测试。重点测试添加/取消/关闭/重复点击/切换协议和地区、重连失败与配置写失败、无权限/超时/限流/重定向、慢中转与旧服务同时刷新、缓存冷启动、全部现有额度供应商及皮肤/缩放布局。真实账号验收需要单独授权。

## 协议来源

- OpenCode Go: https://github.com/anomalyco/opencode/blob/d21b0790a6c146815f28fea604e41e922f009590/packages/console/app/src/routes/zen/go/v1/usage.ts
- MiniMax 官方 CLI: https://github.com/MiniMax-AI/cli/blob/38aba5f26173dd1065b115b221d79c9ddd87c6bb/src/client/endpoints.ts （同提交 schema、types、quota 工具）
- GLM 官方插件: https://github.com/zai-org/zai-coding-plugins/blob/main/plugins/glm-plan-usage/skills/usage-query-skill/scripts/query-usage.mjs
- New API: https://github.com/QuantumNous/new-api/blob/main/controller/token.go
- One API: https://github.com/songquanpeng/one-api/blob/main/controller/billing.go
- Sub2API: https://github.com/Wei-Shaw/sub2api/blob/main/backend/internal/handler/gateway_handler.go
- Qoder 国区 SDK: https://docs.qoder.cn/cli/sdk/cost-usage
- 方舟 Agent Plan: https://docs.volcengine.com/docs/ark/get-afp-usage-api?lang=zh

## 独立复审修正

第二组未参与实现的审查者发现并复核了以下修正：完整展示第三窗口及工具额度、保留中转部署路径、显示限制最紧的余额及范围、移除账号先提交配置、保留失败凭据清理引用、损坏配置下清理失败关闭、限制取消后在途验证数量。新版详情浮层曾触发更新时间布局回归，已缩小为额度环范围并重新运行回归。

环境目录及保存配置已完整查询：只有用户桌面和当前 Linux 云环境，没有隔离 Windows 环境，也没有保存的云环境配置。本轮不连接用户桌面、不安装更多软件；Windows 编译和凭据失败恢复实测是阻断完成验收的剩余条件。现有旧刷新结果可能暂时清除新一轮加载标记，属于既有低优先级展示问题；不会绕过凭据/来源校验或丢失排队刷新。

## Windows main-project validation (2026-11-08/09)

Integrated into the user's main project after verifying the manually downloaded patch SHA-256: `cb2217a2350cb8e6c248c9293464707070c34aec5cbdc83eef108a852b7cae36`. Existing user edits were backed up and preserved. No quota patch file overlapped the user's six modified files. Later concurrent messenger UI tests were preserved as well.

- Existing Rust 1.96.1 MSVC toolchain, Windows SDK and Build Tools only; no software installed. Jobs limited to one. Test data isolated with `DAOZAI_DATA_DIR`.
- `cargo build --locked --bin daozai --target x86_64-pc-windows-msvc`: passed (debug build).
- Quota-focused library tests with `--features record`: 115 passed, 0 failed, 13 ignored. All 34 new adapter tests passed.
- Full `--all-targets --features record --no-fail-fast -- --test-threads=1` run: 808 passed, 1 failed, 86 ignored across targets. Two calendar credential integration tests explicitly skipped to avoid system credential operations.
- The one failure was `ui_tests::ignored_mail_event_hides_stops_reminders_persists_and_restores`, index out of bounds at line 4306. It creates an event at now + 600 seconds but queries today's date. The run and first focused retry occurred before local midnight; the same compiled test passed immediately after midnight with no code change. This is a pre-existing test date-boundary issue, not a clean full-suite pass. The test was not modified because the file contains concurrent user work.
- Source hashes remained unchanged throughout the completed full regression. `git diff --check` passed. Logs and integration backups are in `target/ai-quota-audit-20261108-233602`.

Still not verified: live provider responses using real accounts, native interactive add/cancel/retry flows, Windows Credential Manager write/delete/failure recovery, installer/release build and runtime performance with real accounts. No real keys were accessed. No push, release, running-app replacement or desktop shortcut change was performed. Qoder and Ark remain deferred as described above.