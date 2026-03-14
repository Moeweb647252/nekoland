# External Shell Mode 设计稿

Date: 2026-03-14

## 背景

目标不是让 `nekoland` 自己实现桌面 UI，而是让它作为 compositor / session host 启动并承载一个外部 shell，例如 QuickShell。

这意味着职责边界应当调整为：

- `nekoland`
  - 负责 Wayland compositor、本地输入/输出/窗口/workspace/runtime
  - 负责启动外部 shell 进程并为其提供 session 环境
  - 负责实现外部 shell 真正依赖的 Wayland 协议
- 外部 shell
  - 负责 panel / tray / taskbar / launcher / lock UI 等高层桌面界面
  - 负责处理 `StatusNotifierItem` / `MPRIS` / 通知 / dbusmenu 等 D-Bus 集成

这个分工和 StatusNotifier 规范本身一致：应用导出 item，host/watcher 决定如何显示，外观并不是 item 的职责。QuickShell 也已经内建 system tray 支持，因此不应在 `nekoland` 内部重复实现 `StatusNotifierWatcher` / `StatusNotifierHost`。

## 结论

`nekoland` 不应实现：

- `org.kde.StatusNotifierWatcher`
- `org.kde.StatusNotifierHost`
- `org.kde.StatusNotifierItem`
- 任何面向 tray/menu/media 的 shell UI D-Bus 视图层

`nekoland` 应实现：

- 外部 shell 进程生命周期管理
- 外部 shell 需要的 Wayland 协议宿主能力
- 必要时为 shell 暴露 compositor-specific control plane

## QuickShell 相关约束

### QuickShell 已可承担的职责

QuickShell 官方文档明确包含：

- `StatusNotifierItem compatible system tray clients`
- 基于 `zwlr_layer_shell_v1` 的 panel/window
- 基于 `ext_session_lock_v1` 的 session lock
- 基于 `zwlr-foreign-toplevel-management-v1` 的 toplevel 列表/窗口管理

因此外部 shell 模式下，`nekoland` 不需要把 tray D-Bus 做进 compositor。

### 当前仓库已具备的协议基础

从 [`crates/nekoland-protocol/src`](/home/misaka/Code/nekoland/crates/nekoland-protocol/src) 可以确认当前已有：

- `zwlr_layer_shell_v1`
- screencopy / image copy capture
- `ext_session_lock_v1`
- output management

### 当前仓库缺失的关键协议

就当前代码库看，尚未看到 `zwlr-foreign-toplevel-management-v1` 的协议模块、桥接逻辑或测试。

这意味着如果目标是让 QuickShell 承担 taskbar / window switcher / app overview 一类功能，那么首先要补的不是 D-Bus，而是 foreign toplevel management 协议。

## 目标与非目标

### 目标

- 让 `nekoland` 可以作为 session host 启动一个外部 shell 进程
- 保证外部 shell 自动获得 `WAYLAND_DISPLAY` 等运行时环境
- 补齐外部 shell 真正依赖的 Wayland 协议
- 保持 `nekoland` 自身 UI-neutral，不引入内建 panel/tray/taskbar

### 非目标

- 不在 `nekoland` 中实现 tray UI
- 不在 `nekoland` 中实现 `StatusNotifierWatcher` / `Host`
- 不在第一阶段处理 shell 配置语言或 shell 主题
- 不要求 `nekoland` 替代 QuickShell 的业务集成层

## 目标运行模型

启动顺序应当是：

1. `nekoland` 启动 backend / protocol server / IPC
2. `nekoland` 确认 Wayland socket 可用
3. `nekoland` 启动外部 shell
4. 外部 shell 连接 Wayland 和 session bus
5. 外部 shell 自己创建 layer-shell surface、lock surface、tray model 等

运行时关系应当是：

- compositor 提供 surface / output / focus / toplevel / lock 等协议状态
- shell 作为普通客户端连接 compositor
- shell 与其它桌面服务直接通过 D-Bus 交互

## Shell Integration 与 Viewport 系统的关系

这是外部 shell 模式里一个必须提前定死的边界：

- viewport 属于 output 对 workspace scene 的投影状态
- 外部 shell surface 属于 output-local UI surface
- 两者不能共用一套“窗口几何”语义

### 1. 外部 shell surface 不应跟随 viewport 平移

QuickShell 这类外部 shell 创建的 panel / bar / launcher / lock UI，本质上是：

- `layer-shell` surface
- 或 session-lock surface

这些 surface 应当锚定在 output-local 坐标系中，而不是 workspace scene 中。

因此：

- viewport 平移不能让 panel/tray/taskbar 跟着移动
- shell UI 不能被当作普通 workspace window 参与 scene projection
- shell UI 不应写入普通窗口的 scene-space 几何模型

换句话说：

- 普通窗口：`scene geometry -> viewport projection -> presentation geometry`
- shell UI：直接是 `output-local presentation geometry`

### 2. 外部 shell 看到的 toplevel 几何默认应是 presentation 语义

如果外部 shell 通过 foreign toplevel management 观察窗口，它做 taskbar / alt-tab /
window list 的第一目标是“当前屏幕上这个窗口表现成什么样”，而不是“窗口在无限 workspace 上的真实 scene 坐标”。

因此默认规则应当是：

- foreign toplevel 等面向 shell 的实时窗口管理协议，优先暴露当前 presentation /
  screen 语义
- 即：
  - 当前 output 可见性
  - 当前 output-local 或 screen-space 几何
  - 当前激活 / 最大化 / 全屏等状态

不要让外部 shell 从第一版开始默认消费 scene-space 几何，否则它会把 viewport
投影细节错误地当成“UI 摆放坐标”。

### 3. 如果 shell 要做 pager / overview，必须显式拿 viewport 状态

有些外部 shell 不只做 taskbar，还会做：

- workspace pager
- minimap / overview
- “定位到窗口”按钮
- 当前相机位置指示器

这些能力就不能只看 presentation 几何，必须显式知道：

- 每个 output 当前显示哪个 workspace
- 每个 output 当前的 viewport origin
- 窗口的 scene-space 坐标

这意味着如果以后给外部 shell 增加 compositor-specific bridge，那么 bridge 的几何语义
必须明确区分：

- `scene_x/scene_y`
- `screen_x/screen_y`
- `output.current_workspace`
- `output.viewport_origin`

不能再退回到一组匿名 `x/y`。

### 4. 外部 shell 的“主语”应当是 output，而不是全局 active workspace

viewport 系统已经把“看哪块 scene”明确收敛到 output-scoped 状态，因此外部 shell
集成也必须遵守这个主语：

- panel / bar / lock surface 绑定 output
- output hotplug 触发 shell UI 的 output-local 重建
- 如果 shell 做 workspace UI，它应当先按 output 取 current workspace，再解释 viewport
- 不应再假设“全局只有一个 active workspace”

### 5. 对控制桥的约束

如果后续增加 shell-facing control bridge，那么 bridge 中所有 viewport 相关动作都必须
是 output-scoped：

- `move_viewport_to(output, x, y)`
- `pan_viewport_by(output, dx, dy)`
- `center_viewport_on_window(output, surface_id)` 或明确的 focused-output fallback

不应设计成：

- 全局唯一 viewport
- workspace 直接拥有唯一 camera
- shell surface 通过移动自己来模拟 viewport

### 6. 实施上的直接影响

这会直接影响外部 shell 模式的 Phase 2 和 Phase 4：

- Phase 2 `zwlr-foreign-toplevel-management-v1`
  - 要明确哪些状态走 presentation 语义
  - 不把 viewport 投影细节泄漏成 shell 必须理解的默认模型
- Phase 4 compositor-specific control bridge
  - 如果要支持 pager / overview / “转到窗口”，必须显式补 output + viewport 查询与控制

因此，外部 shell 模式不是独立于 viewport 系统的一条平行线，它必须建立在已经稳定的：

- scene-space / presentation-space 拆分
- per-output current workspace
- per-output viewport

这三层基础之上。

## 配置模型

建议在配置中新增一组明确的外部 shell 配置，而不是复用普通 startup command：

```toml
[shell]
enabled = true
argv = ["quickshell", "-p", "/path/to/shell.qml"]
restart = "on-failure"
startup_timeout_ms = 5000

[shell.environment]
QT_QPA_PLATFORM = "wayland"
QT_WAYLAND_DISABLE_WINDOWDECORATION = "1"
```

语义约束：

- `shell.enabled = false` 时，`nekoland` 仍可作为“无内建 shell”的纯 compositor 运行
- `shell.argv` 是唯一的 shell 入口，不和一般 startup commands 混用
- shell 环境继承 compositor 进程环境，但可叠加显式覆盖项

## 启动与生命周期设计

### 1. Shell Launcher Resource

新增一个 shell runtime 资源，负责：

- 当前 shell 子进程 pid / handle
- 最近一次启动时间
- 退出码 / 失败原因
- restart policy 状态

建议放在一个独立 crate 或至少独立模块中，不混进 input / shell layout 逻辑。

### 2. 启动时机

启动条件必须至少满足：

- Wayland socket 已创建
- 若启用 XWayland 并希望 shell 消费 `DISPLAY`，则 XWayland 已 ready

否则会出现 shell 抢先启动、拿不到完整环境的问题。

### 3. 环境注入

至少注入：

- `WAYLAND_DISPLAY`
- `XDG_RUNTIME_DIR`
- `XDG_CURRENT_DESKTOP=nekoland`
- 若 XWayland ready，则注入 `DISPLAY`
- 用户配置的额外环境变量

不建议由 compositor 伪造 session bus；应直接继承用户 session 的 `DBUS_SESSION_BUS_ADDRESS`。

### 4. 重启策略

第一版只建议支持：

- `never`
- `on-failure`
- `always`

并增加退避，避免 shell 崩溃时进入热重启死循环。

## Wayland 协议实施顺序

### Phase 1: External Shell Launcher

先不补新协议，只完成：

- 外部 shell 配置
- shell 子进程启动/停止/重启
- shell 状态资源与日志
- Wayland/XWayland 环境注入

退出条件：

- 可以稳定启动 QuickShell 这类外部客户端
- shell 作为普通 layer-shell 客户端显示 panel

### Phase 2: Foreign Toplevel Management

补 `zwlr-foreign-toplevel-management-v1`：

- 导出当前 toplevel 列表
- 活动窗口状态
- 标题 / app id / maximize / minimize / fullscreen / activate / close
- 窗口与 output/workspace 的可见性关系

这是 QuickShell 做 taskbar / window switcher 的核心依赖。

退出条件：

- 外部 shell 能枚举和观察 toplevel
- 基本窗口控制请求能回到 compositor

### Phase 3: Shell-Facing Session Capabilities Hardening

在 launcher + toplevel 协议跑通后，再补壳层真正需要的剩余能力：

- session lock 行为回归
- screencopy / capture 权限边界
- output metadata / hotplug 稳定性
- keyboard focus rules for shell surfaces

退出条件：

- 外部 shell 可实现 panel + taskbar + lock screen 的基础桌面体验

### Phase 4: Optional Compositor-Specific Control Bridge

仅当 QuickShell 需要调用 `nekoland` 特有动作时，再增加一条 shell-facing control plane。

这条桥不一定是 D-Bus；更适合优先复用现有高层控制面语义，做一条清晰、版本化的专用接口。

## crate 拆分建议

### 方案 A: 新增 `crates/nekoland-shell-host`

职责：

- 读取 shell 配置
- 管理子进程生命周期
- 输出 shell 状态事件

优点：

- 和现有 `nekoland-shell` 的窗口管理逻辑不混
- 容易单测

### 方案 B: 暂时挂在 `nekoland-core` 或主二进制

仅适合第一阶段快速落地，不建议长期保留。

建议优先采用方案 A。

## 测试计划

### 单测

- shell 配置解析
- shell 启动环境拼装
- restart policy / backoff
- XWayland ready 前后的启动门控

### 集成测试

- compositor 启动后自动拉起一个测试 shell 进程
- 测试 shell 能成功连接 Wayland socket
- layer-shell panel 能进入 ECS / render list
- shell 崩溃后按策略重启

### 协议测试

- foreign toplevel manager 导出当前窗口列表
- 活动窗口变化会发出协议更新
- 外部 shell 发起 activate / close / fullscreen 请求后，控制面能正确入队

## 风险

### 1. 把 tray/D-Bus 做进 compositor会造成职责反转

这会让 `nekoland` 同时承担 compositor 和 desktop shell 两层职责，和“外部 shell 模式”目标冲突。

### 2. 启动时序不对会导致 shell 拿不到完整环境

特别是 `WAYLAND_DISPLAY` / `DISPLAY` / session bus 地址。

### 3. foreign toplevel 协议是外部 shell 可用性的硬门槛

没有它，QuickShell 可以画 panel，但做不好 taskbar / window list。

## 建议的最近执行顺序

1. Phase 1: 外部 shell launcher
2. Phase 2: `zwlr-foreign-toplevel-management-v1`
3. Phase 3: shell-facing capability hardening
4. 只有在确有需要时，才考虑 compositor-specific D-Bus / IPC bridge

## 参考资料

- QuickShell About:
  https://quickshell.org/about/
- QuickShell `WlrLayershell`:
  https://quickshell.org/docs/master/types/Quickshell.Wayland/WlrLayershell/
- QuickShell `ToplevelManager`:
  https://quickshell.org/docs/v0.2.1/types/Quickshell.Wayland/ToplevelManager
- QuickShell `WlSessionLock`:
  https://quickshell.org/docs/master/types/Quickshell.Wayland/WlSessionLock/
- QuickShell `SystemTray` / `SystemTrayItem`:
  https://quickshell.org/docs/types/Quickshell.Services.SystemTray/
  https://quickshell.org/docs/types/Quickshell.Services.SystemTray/SystemTrayItem/
- Status Notifier Item spec:
  https://specifications.freedesktop.org/status-notifier-item/0.1
