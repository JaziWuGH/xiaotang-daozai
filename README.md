# Jazi 定制岛仔（个人部署）

基于上游 [Renly1994/xiaotang-daozai](https://github.com/Renly1994/xiaotang-daozai) **v0.6.61**，叠了自己的改动，方便换机直接装，不跟上游自动同步。

## 相对上游多了什么

- 播放控制：后台模式切换、封面开关式显隐网易云、WASAPI 音量滑动条、空态点封面选播放器
- 歌词：合唱曲歌手匹配放宽 + cloudsearch 接口
- 手势：触控板 / 触摸屏左右滑切页
- 诊断：heartbeat / last-exit 日志

## 换机快速安装（Windows）

1. 打开本仓库 [Releases](https://github.com/JaziWuGH/daozai/releases)，下载最新 `Daozai-windows-x64.zip`
2. 退出正在运行的岛仔
3. 解压后把 `daozai.exe` 覆盖到：

   `%LOCALAPPDATA%\Programs\Daozai\daozai.exe`

   若还没装过官方版，先建这个目录，或把 exe 放到任意目录后创建快捷方式。
4. 双击运行 `daozai.exe`

也可以在 PowerShell 里跑仓库根目录的 `install.ps1`（会下载最新 Release 并覆盖安装）。

## 从源码构建

需要 Rust stable + Visual Studio C++ 工具链。

```powershell
cargo build --release
Copy-Item .\target\release\daozai.exe "C:\Users\Administrator\AppData\Local\Programs\Daozai\daozai.exe" -Force
```

## 说明

- 本仓库是个人定制快照，**不会**自动跟踪上游；要吸上游再手动 merge。
- 上游共创 PR 仍在 fork `JaziWuGH/xiaotang-daozai`（#15 / #26）。
- 版权与许可跟随上游项目。
