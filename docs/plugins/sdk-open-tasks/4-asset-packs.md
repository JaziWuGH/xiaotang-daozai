# 第 4 步：皮肤、表情、图标资源包

开放不含代码的资源包，皮肤必须做，表情和图标看现状能做多少做多少。分支用 `feat/plugin-asset-packs`，共同规则见 [README](README.md)。

## 先摸清现状

- **内置皮肤**：在 `src/` 里搜 skin 和 theme，看皮肤怎么定义、保存和切换。重点看三处：
  - `src/app.rs`
  - `src/extras.rs` 的外观设置页
  - `src/model.rs` 的配置保存
- **表情与图标**：看仓库里有没有「表情」或桌宠素材，以及模块图标从哪里来（例如 `assets/settings-icons`）。
- **已有的资源型插件**：参考 `navigation.shortcuts` 是怎么接的，涉及三处：
  - `spike/local.rs` 的 `is_page_shortcuts`
  - `spike/local.rs` 的 `validate_page_shortcut_colors`
  - `spike/capabilities.rs` 里的 `Role::Exclusive`

## 要做的

### `appearance.skin`（必须做）

**清单与包内容**

- 清单写 `runtime:"assets"`、`owner:"appearance"`，这是独占能力。入口文件为 `theme.json`。
- 只开放现有皮肤结构里能安全替换的配色令牌，颜色一律用 `#RRGGBB`。未列出的令牌回退到所选的基础内置皮肤。
- 包里不能有脚本，文件大小设上限。

**用户怎么用**

- 安装并启用后，插件皮肤出现在外观的皮肤选择器里，旁边带「插件」小标。
- 同一时间只能选用一个插件皮肤。
- 皮肤形状（胶囊或刘海）和停靠位置仍由用户掌管，插件不改。
- 停用或卸载正在使用的插件皮肤时，回到用户上次用的内置皮肤，期间用户改过的形状保留。

### `appearance.icon`（看现状决定）

- 模块图标有统一入口的话，允许用 SVG 图标包替换部分模块图标，缺的回退到内置图标。
- SVG 要校验：设大小上限，不能有脚本、外链或 foreignObject。
- 没有统一入口、要大改才能接的，就不做，在回报里说明原因。

### `appearance.pet`（表情）

仓库里没有表情或桌宠系统的话，不要新造一套，在回报里说明。

### 收尾

- 在 `CAPABILITIES` 里补上对应的行。
- 在 `spike/examples/` 加一个皮肤包样例，封面可以复制 payday 的。
- 更新文档：
  - `docs/plugins/sdk-v1-design.md` 里皮肤、表情、图标那一行；
  - 同一文件第 4 节的换肤场景。

## 验收标准

- 单元测试至少覆盖：
  - `theme.json` 校验：坏颜色、未知令牌、超大文件、带脚本。
  - 能力组合。
  - 换肤场景：从奶油换成插件皮肤，期间把胶囊改成刘海，停用插件后显示为奶油加刘海。
  - 同一时间只能选用一个插件皮肤。
  - 卸载后回退到内置皮肤。
- 下面这条命令全部通过，你改到的外观相关定向测试也全部通过：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File spike/cargo-serial.ps1 test --locked --lib --features "sdk-js sdk-material" --target x86_64-pc-windows-msvc sdk_spike
```

- 回报里写清哪些能力做了，哪些没做，没做的说明原因。
