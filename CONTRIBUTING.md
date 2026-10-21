# 贡献说明

欢迎提交问题或改进。结构性改动请先在 Issue 中说明预期行为与范围。

安装 Node.js、Rust、Visual Studio C++ 构建工具、Windows SDK 和 WebView2，按 [README](README.md#从源码构建) 启动开发环境。

## 代码位置

| 路径 | 内容 |
| --- | --- |
| `script-manager/src/` | React 主界面、标题栏、托盘菜单与样式 |
| `script-manager/src-tauri/src/lib.rs` | 应用初始化、托盘及窗口管理 |
| `script-manager/src-tauri/src/scripts.rs` | 条目校验、编辑与配置保存 |
| `script-manager/src-tauri/src/runner.rs` | 进程运行、停止、暂停、输入与输出 |

Rust 使用 snake_case，JavaScript 使用 camelCase。保持代码直观可读，优先使用现有依赖。界面文案使用简体中文，协议字段和脚本原始输出保持原样。

## 提交前验证

1. 在 `script-manager/` 运行 `npm run build`。
2. 在 `script-manager/src-tauri/` 运行 `cargo fmt --check` 和 `cargo check --locked`。
3. 运行程序验证改动；涉及进程时，使用独立的通用测试脚本检查启动、输出和停止。
4. 同步相关 README 和版本记录，检查提交中没有真实配置、私人地址、凭据、日志、备份或构建机路径。

提交只包含当前改动。示例使用明显的通用值；公开截图也要检查是否露出本机用户名、服务名称、命令、路径及输出。

## 问题反馈

请提供应用版本、Windows 版本、操作步骤、预期行为与实际结果。脚本问题请提供最小示例，先移除私人信息。安全问题参见 [SECURITY.md](SECURITY.md)。
