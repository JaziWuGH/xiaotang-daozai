# 本地试装与测试集市

在 Windows 上，用启用了 `sdk-js` 的岛仔打开「设置 → 插件」。已在使用岛仔时，可先从托盘退出当前实例，再启动试用版。可以双击[试用本地插件.cmd](../../spike/试用本地插件.cmd)编译并启动。等价的 PowerShell 命令在工作树根目录运行，试用数据放进 `target/local-trial`。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File spike/cargo-serial.ps1 build --locked --features sdk-js --bin daozai --target x86_64-pc-windows-msvc
$env:DAOZAI_DATA_DIR = Join-Path (Get-Location) 'target/local-trial'
& './target/x86_64-pc-windows-msvc/debug/daozai.exe'
```

1. 在「已安装」底部点「开发者工具」，再点「安装本地插件…」，选择[发薪倒计时清单](../../spike/examples/payday/plugin.json)。安装后打开它的开关。用相同步骤装上[旅行倒计时](../../spike/examples/trip/plugin.json)。
2. 点发薪插件下方的路径「设置 › 待办与置物 ›」，在「待办与置物」修改标题和目标日期并保存。展开岛上的待办页，在原有待办列表下方看卡片。有两张卡时，点卡片可切换。
3. 停用旅行插件，只留下发薪卡。重启同一试用版，旅行仍为停用。关闭「页面管理 → 待办」停止所属插件的刷新，重开待办后仅发薪卡恢复。
4. 卸载发薪插件并选「保留数据」。从原目录重装并启用，核对上次保存的日期。再卸载并选「删除数据」，重装后应显示样例默认日期。旅行插件独立保存自己的设置和状态。

## 作者如何接入

复制一个样例目录，在 `plugin.json` 改插件 ID、名称和设置默认值。宿主按清单的 `entry` 加载 JS。文本与日期字段由清单声明，插件在 `refresh({ settings, today, storage })` 中得到已保存的设置、本机日期和上次提交的私有 JSON 数据。返回的 `card` 是显示文字，`storage` 是本次要保存的快照。无外部权限。此试用实现支持归属待办的本地卡片，刷新由宿主调度。

把封面保存为包根目录的 `cover.png`，并在清单写入 `"cover": "cover.png"`。PNG 文件最大 256 KiB，宽高不超过 1920×1080，建议使用 16:9。缺图或图片无效时，岛仔会给出校验错误。

安装将清单、入口脚本和封面复制到本地记录，后续独立于源码目录运行。要试修改后的包，可选择保留数据卸载，从原清单路径重装。同一 ID 已安装时会提示重复。保留数据用 ID 和本地清单路径识别，仅适用于本人挑选的开发包。默认数据在岛仔配置目录的 `local-plugins` 子目录。

每个包用同一份清单声明 `timer` 和 `refreshSeconds`（60 到 86400 秒，省略时为 60）。每次刷新都从头执行 main.js；下次还要用的值请放进返回的 storage。停用、关闭待办或改完设置后，上一次计算即使返回也不会再显示或保存。action（按钮）和 event（事件）仍待宿主实现。

重启时岛仔重新核对已安装记录的 ID、清单、脚本入口和设置。看到“需要重新安装”时，在「开发者工具」重新选择可信的原包，或点「移除记录」。未通过检查的记录不会启动；重装会使用原包的默认设置。

需要显示进度、指标和告警的插件可使用[结构化状态页](sdk-v1-design.md#结构化状态卡和详情页)。安装并启用插件后，在「设置 → 页面管理 → 状态详情」可打开或暂停该页。授权文件和本机接口在插件的已安装详情中管理。

### 快捷切换插件

包根目录放置 `plugin.json`、`shortcuts.json` 和 `cover.png`。清单使用 `runtime: "assets"`、`owner: "pages"`、`entry: "shortcuts.json"`、`capabilities: ["navigation.shortcuts"]`、`permissions: {}` 和 `settings: []`。

在 `shortcuts.json` 的 `colors` 中至少填一个页面的选中颜色。颜色使用六位 `#RRGGBB`。页面键支持 `music`、`focus`、`health`、`shelf`、`tasks`、`mail`、`quota`、`calendar`、`weather`、`performance` 和 `wireless`。

```json
{"colors":{"music":"#FA3E72","focus":"#FF8A30","weather":"#4C9BE8"}}
```

安装后在「插件 → 已安装」启用，再到「设置 → 页面管理」调整快捷切换图标的显示。图标按岛仔的页面顺序排列，点击可切换到对应页面。停用或卸载后恢复原有页码点。

## 测试集市

测试集市使用本地的 `catalog.json` 和同目录 ZIP。用另一个插件 ID 或干净的试用配置做测试，方便分别验证本地安装和集市的安装来源。

### 作者校验、打包和提交

复制[发薪倒计时样例](../../spike/examples/payday)到可编辑目录。用岛仔试用版执行三条命令。下面的示例从工作树根目录运行，以已编译的 Debug 程序为例。

```powershell
$island = (Resolve-Path './target/x86_64-pc-windows-msvc/debug/daozai.exe').Path
$manifest = (Resolve-Path './spike/examples/payday/plugin.json').Path
$bundle = Join-Path (Get-Location).Path 'target/payday-trial.zip'
$catalog = Join-Path (Get-Location).Path 'target/test-market/catalog.json'
New-Item -ItemType Directory -Force (Split-Path $catalog) | Out-Null

& $island --plugin-test $manifest | Out-Host
& $island --plugin-pack $manifest $bundle | Out-Host
& $island --plugin-submit $bundle $catalog | Out-Host
```

`--plugin-test` 检查清单、入口和封面。JavaScript 插件还会用默认设置试跑一张卡片。`--plugin-pack` 导出含封面的 ZIP。`--plugin-submit` 将已验证的包放进测试目录，随后更新索引。命令失败时查看输出，并在修正后重新执行。

支持 GPU 材质的 Windows 版也可以按同样步骤打包[液态玻璃样例](../../spike/examples/liquid-glass)。`.wgsl` 入口须在包根目录并通过着色器校验。安装后保持停用，首次启用时需要确认屏幕底片权限，并开启 Direct3D 12 兼容渲染。

提交到本地测试目录后，可以用 `--plugin-avatar <目录索引> <插件 ID> <GitHub 用户名>` 同步作者头像。例如 `& $island --plugin-avatar $catalog cayww.liquid-glass cayww`。同步后刷新测试集市目录即可查看，插件沿用原版本和 ZIP。

要试更新，把样例副本的 `version` 从 `1.0.0` 改成 `1.0.1`，同时给卡片文案加一处能看出版本差异的改动。保持插件 ID、设置键和数据版本。用新清单、新 ZIP 文件路径再次校验和打包，然后提交到同一个 `catalog.json`。

### 用户安装和更新

1. 在「设置 → 插件 → 已安装 → 开发者工具」点「导入测试集市目录…」，选择 `catalog.json`。导入后集市切到「本地测试」，右上角可以在「官方」和「本地测试」之间切换。官方集市优先使用本机缓存，缺少缓存时自动加载，可点击刷新按钮手动同步目录和统计。
2. 在本地测试集市找到插件并点「安装」，切换到「已安装」打开开关。
3. 点插件下方的路径或岛上插件卡片的小齿轮，在「待办与置物」修改名称和日期。停用后，这一组插件设置和卡片收起。暂停待办模块时，开关保留原启用选择，路径变成琥珀色的「页面管理 › 待办与置物 ›」。
4. 作者提交更高版本后，在同一个测试目录点「刷新目录」和「更新」。重新打开卡片核对新文案，原名称、日期和启用状态仍在。
5. 卸载时选择保留数据，从原测试目录重装同 ID 插件并启用，核对原设置。选择删除数据时，下次安装使用样包默认值。
