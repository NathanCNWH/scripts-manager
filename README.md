# 脚本管理器（Scripts Manager）

<p align="center"><img src="assets/logo.png" alt="脚本管理器图标" width="96"></p>

一个轻量的 Windows 托盘工具，用一个窗口集中管理本机脚本和命令，查看运行状态、实时输出并发送交互输入。

- **界面语言**：简体中文
- **支持平台**：Windows 10 / 11，x64
- **当前版本**：0.1.1
- **技术栈**：Tauri 2、Rust、React 19、Vite 7
- **许可**：[MIT](LICENSE)，基于 [WassBe/scripts-manager](https://github.com/WassBe/scripts-manager) 的改进版本，保留原作者版权

## 下载与安装

到 [发行版本](https://github.com/NathanCNWH/scripts-manager/releases/latest) 下载：

| 文件 | 用途 |
| --- | --- |
| `scripts-manager_0.1.1_windows_x64_setup.exe` | 中文安装程序，按向导安装 |
| `scripts-manager_0.1.1_windows_x64_portable.zip` | 便携程序，解压后运行 `script-manager.exe` |
| `SHA256SUMS.txt` | 下载文件的 SHA-256 校验值 |

需要 Microsoft Edge WebView2 运行时。安装版会在缺失时引导安装；便携版需要系统已经安装该运行时。便携版无需安装管理器，但仍将脚本清单保存在用户应用数据目录中。

启动后，程序驻留在任务栏右侧托盘。**左键**点击图标显示或隐藏主窗口，**右键**打开菜单。主窗口右上角关闭按钮会隐藏到托盘；在托盘菜单中点击“退出”才会退出管理器。

## 主要功能

- 导入或拖入 Python、PowerShell、批处理、Node.js、Shell 脚本及可执行程序：`.py`、`.ps1`、`.bat`、`.cmd`、`.js`、`.sh`、`.exe`。
- 直接添加命令，选择 `cmd`、`pwsh` 或 `bash`，指定可选的工作目录。
- 启动、停止、暂停和继续运行中的脚本，用状态标记区分运行、暂停和停止。
- 在内置终端查看标准输出和错误输出，按脚本筛选，并向运行中的脚本发送输入。
- 编辑条目名称、启动设置、工作目录和自定义停止文件或命令。
- 打开 JSON 配置文件；外部编辑保存后，程序约每两秒刷新条目。
- 拖动标题栏移动主窗口，拖动四边或四角调整大小；同一次运行中，隐藏后重新打开保留位置和尺寸。
- 深浅色主题、托盘菜单和可选的开机启动。

管理器只保存条目定义，不会复制、改写或删除你的原始脚本文件。

## 基本使用

1. 点击“导入文件”选择脚本，或点击“添加”填写文件或命令条目。
2. 点击条目右侧的运行按钮；悬停在图标按钮上可查看中文说明。
3. 点击底部“终端”查看输出。选中脚本后，在输入框中输入内容并按 Enter 发送。
4. 点击编辑按钮修改条目；右键菜单可以移除条目。移除正在运行的条目前会要求确认并停止该脚本。

编辑运行中的条目时，启动设置在下次运行时生效；停止设置在下次点击停止时读取。

## 停止操作

文件按扩展名选择解释器；命令通过 `cmd /C`、`pwsh -NoProfile -Command` 或 `bash -lc` 执行。工作目录同时作用于启动与停止操作。未填写时，文件使用其所在目录，命令使用管理器所在目录。

未配置停止操作时，停止会结束管理器启动的进程树。配置停止文件或命令时，管理器先执行该操作，最多等待 10 秒，再给被管理进程最多 5 秒退出；仍未退出时结束进程树。“停止全部”和移除条目使用相同规则。

停止操作会收到环境变量 `SCRIPT_MANAGER_PID`。例如，cmd 停止命令可使用 `taskkill /PID %SCRIPT_MANAGER_PID% /T /F` 指向该条目的进程树。

对于管理器之外启动的服务，管理器不会自动接管进程；有停止操作的条目仍可点击“执行停止操作”。此时 `SCRIPT_MANAGER_PID` 为 `0`，停止命令应根据服务自身标识定位目标，不能直接使用上述按 PID 停止的示例。

## 配置文件

点击“打开配置”，用记事本编辑真实的 `scripts.json`。主界面显示完整本机路径。默认位置为 `%APPDATA%\com.wassbe.script-manager\scripts.json`。应用标识沿用上游值以兼容已有配置，不表示此分支由上游维护。

以下是通用示例，路径需要自行替换。不要将真实配置提交到仓库：

```json
[
  {
    "id": "example-service",
    "name": "示例服务",
    "mode": "command",
    "command": "node server.js",
    "shell": "cmd",
    "working_dir": "C:\\scripts\\example-service"
  },
  {
    "id": "example-file",
    "name": "示例脚本",
    "mode": "file",
    "path": "C:\\scripts\\start.ps1",
    "stop": { "path": "C:\\scripts\\stop.ps1" }
  }
]
```

- 新条目可省略 `id`，管理器会生成并保存；编辑既有条目时保持编号稳定。
- 旧的 `id`、`name`、`path` 三字段条目仍可读取。
- 配置格式错误时会报错，并保留最后一次成功加载的列表；修正保存后会自动刷新。
- 修改配置不会自动启动服务。

## 从源码构建

需要 Node.js 20.19+ 或 22.12+、Rust 稳定版、Visual Studio C++ 构建工具及 Windows SDK。

```powershell
git clone https://github.com/NathanCNWH/scripts-manager.git
cd scripts-manager/script-manager
npm ci
npm run tauri dev
```

检查及构建：

```powershell
npm run build
cd src-tauri
cargo fmt --check
cargo check --locked
cd ..
npm run tauri build -- --bundles nsis
```

安装程序位于 `script-manager/src-tauri/target/release/bundle/nsis/`，独立程序为 `script-manager/src-tauri/target/release/script-manager.exe`。公开发行只打包程序及许可证，不打包 `.pdb`、真实配置、日志、备份、依赖缓存或构建目录；建议通过 Rust `--remap-path-prefix` 移除构建机的个人路径。

## 使用边界

- 脚本和命令使用当前用户权限运行，不提供沙箱隔离。只运行可信脚本，并只允许可信程序编辑配置。
- 暂停会直接冻结进程，脚本不会主动感知暂停，连接可能超时。
- 解释器需要可从 `PATH` 找到。`pwsh` 指 PowerShell 7，需要单独安装。
- 输出只保留在管理器内存中，最多 500 行；原始内容不会自动脱敏。
- 本版本保留同一次运行中的窗口位置和尺寸，暂不跨退出重启保存。

## 项目资料

- [版本记录](CHANGELOG.md)
- [贡献说明](CONTRIBUTING.md)
- [安全与隐私说明](SECURITY.md)
- [第三方及上游声明](NOTICE)
- [问题反馈](https://github.com/NathanCNWH/scripts-manager/issues)
