# 参与开发

欢迎反馈兼容性问题、修复 bug 或完善使用说明。提交较大的功能前，先开 Issue 说明具体场景与预期行为。

## 本地开发

安装 Rust stable、Visual Studio C++ 构建工具和 Windows SDK，克隆仓库后运行：

```powershell
cargo +stable-x86_64-pc-windows-msvc build --locked --bin daozai --target x86_64-pc-windows-msvc
cargo +stable-x86_64-pc-windows-msvc test --locked --all-targets --features record --target x86_64-pc-windows-msvc -- --test-threads=1
```

为开发实例指定独立目录，避免覆盖日常使用数据：

```powershell
$env:DAOZAI_DATA_DIR = Join-Path $env:TEMP 'daozai-development'
cargo +stable-x86_64-pc-windows-msvc run --locked --bin daozai --target x86_64-pc-windows-msvc
```

常规测试使用合成数据。带 `#[ignore]` 的真实账号测试或截图导出测试须按需要显式运行；不要在公共 CI 中连接私人账号。

## 任务与版本流程

1. 从最新 `main` 为一个独立问题建立任务分支，例如 `fix/wechat-tray`。记录复现方式、预期行为和验收条件。同一问题续修沿用分支；并行任务使用不同 worktree。
2. 完成一个完整修复并验证后提交，标题说明具体问题。修复代码与对应测试一起提交，用户可感知的改动同步写入 `CHANGELOG.md` 的「未发布」条目。
3. 将任务分支推送至 GitHub 并创建 PR，说明问题、结果和验证。审查整个分支相对目标分支的差异；审查后继续修改时，追加提交并重新检查受影响部分。
4. Windows CI 检查通过、问题完成审查后合并到 `main`。一次正式版可以积累多个已经完成的任务；提交和合并不要求立即发版。
5. 正式发布时确定版本号，将「未发布」内容整理到该版本，核验最终提交的测试与安装包，再关联同一提交的 Tag 和 GitHub Release。已发布 Tag 保持不变。

收到外部 PR 时，先确认问题和改动范围，在独立工作区检出并验证，再决定接受、请求修改或关闭。合并只更新源码；当前 CI 执行测试、打包验证与安装测试，不会自动创建正式 Release。

## 查看未发布内容

以 GitHub 最新正式 Release 对应的 Tag 为基线，结合提交历史与最终代码差异，检查主分支、任务分支和本地未提交内容。未合并不代表未提交，已经提交也可能仍未发布。其他活动分支或 worktree 中的工作须单独列出。

例如上一正式版是 `v0.6.63` 时，查看当前分支：

```powershell
git log --oneline v0.6.63..HEAD
git diff --stat v0.6.63 HEAD
git status --short
```

可以直接要求：「汇总岛仔相对上一正式版的全部未发布改动，按功能列出，并区分已合并、未合并、未提交，说明验证结果和待验收事项。」

## 代码导航

| 目录／文件 | 内容 |
| --- | --- |
| `src/main.rs` | Windows 外壳、系统状态和后台服务衔接 |
| `src/app.rs`、`src/pages.rs` | 状态与主界面 |
| `src/calendar.rs`、`src/calendar_ui.rs` | CalDAV、重复日程、月历与列表 |
| `src/mail.rs`、`src/mail_reader.rs` | IMAP、MIME、验证码与邮件阅读 |
| `src/quota*.rs` 及 provider 模块 | AI 额度与连接 |
| `src/native.rs`、`src/system.rs` | 原生窗口、输入、拖放和系统事件 |
| `src/ui_tests.rs` | 无窗口交互与布局回归测试 |
| `src/bin/daozai-record` | 合成数据演示与软件渲染器 |
| `vendor/` | 少量必要的上游补丁，见依赖说明 |
| `installer/`、`scripts/` | 安装、卸载与打包 |

## 提交要求

修 bug 时提供能复现的测试；界面变更附不含私人信息的截图。保持改动聚焦，不把个人配置、授权码、Cookie、编译产物或真实账号数据提交到仓库。

源码贡献按仓库 MIT 许可提交；第三方素材须同时提交准确的来源与许可声明。不要修改或移除 vendored 依赖的原始版权声明。
