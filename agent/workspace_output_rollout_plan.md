# Workspace / Output 重构实施计划

Date: 2026-03-14

## 范围

这份计划把以下三份设计稿收敛成一个有明确先后依赖的实施路线：

1. [`agent/infinite_workspace_viewport_design.md`](/home/misaka/Code/nekoland/agent/infinite_workspace_viewport_design.md)
2. [`agent/per_output_active_workspace_design.md`](/home/misaka/Code/nekoland/agent/per_output_active_workspace_design.md)
3. [`agent/output_background_design.md`](/home/misaka/Code/nekoland/agent/output_background_design.md)

目标不是一次性同时落地三套模型，而是按基础依赖从下往上迁移，确保每个阶段结束时系统都还能稳定运行和测试。

## 顺序总览

### Phase 1: Infinite Workspace Viewport Foundation

先完成 scene-space / presentation-space 拆分，以及 output viewport 基础控制。

为什么必须先做：

- 用户要求窗口位置能进入 `isize` 级别的无限 workspace 范围
- 当前 `SurfaceGeometry` 同时承担逻辑位置和屏幕位置，必须先拆
- 后续 per-output workspace 和 output background 都要建立在“scene 与 output-local 是两层语义”这个前提上

### Phase 2: Per-Output Active Workspace

在 viewport foundation 稳定后，把“当前显示哪个 workspace”从全局状态迁到 output。

为什么排第二：

- viewport 本身就是 output-scoped 状态
- 如果不先解决 scene / projection，per-output workspace 仍然会卡在单份几何语义上
- 完成这一阶段后，output 才真正拥有“看哪个 workspace、看 scene 的哪一块”两层完整状态

### Phase 3: Output Background Window

最后落地 output background role。

为什么排最后：

- 它依赖 output-aware render filtering
- 它要求普通 workspace scene 与 output-local UI surface 的边界已经明确
- 只有在 per-output workspace 和 viewport 语义清晰后，background 才不会和普通窗口场景混淆

## Phase 1: Infinite Workspace Viewport Foundation

对应设计稿：

- [`agent/infinite_workspace_viewport_design.md`](/home/misaka/Code/nekoland/agent/infinite_workspace_viewport_design.md)

### 目标

- 让普通窗口拥有稳定的 scene-space 逻辑坐标
- 把 `SurfaceGeometry` 收敛为 presentation/output-local 几何
- 为 output 引入 viewport origin
- 支持 viewport 手动平移和“定位到窗口”

### 子阶段

#### 1A. 坐标模型拆分

- 新增 `WorkspaceCoord = isize`
- 新增 `ScenePoint` / `SceneRect` 或等价命名的 scene-space 类型
- 给普通窗口新增 scene-space 几何来源
- 让 `WindowPlacement` / `WindowRestoreSnapshot` 的位置语义转向 scene-space

#### 1B. Viewport 基础状态与控制面

- 给 output 增加 `OutputViewport { origin }`
- 在 `PendingOutputControls` / `OutputOps` 增加：
  - `pan_viewport_by(dx, dy)`
  - `move_viewport_to(x, y)`
  - `center_viewport_on_window(surface_id)`
- keybinding 和 IPC 增加 viewport 控制入口

#### 1C. 投影与消费方迁移

- 增加统一的 scene -> presentation projection helper
- render 组合改为消费投影后的 presentation geometry
- pointer hit-test / keyboard focus fallback / protocol pointer focus / damage tracking 全部切到 presentation geometry
- viewport 外窗口从可见候选中排除，但不改 `WindowMode`

### 本阶段不做

- 不移除全局 `Workspace.active`
- 不重做 per-output workspace 绑定
- 不让 output background role 混入这次实现
- tiled / maximized / fullscreen 继续保持现有 output-local 语义，先不扩展成完整 scene object

### 退出条件

- floating 窗口的逻辑位置不再受 `i32` 屏幕坐标限制
- viewport 平移不会修改窗口 scene 坐标
- “定位到窗口”通过 viewport 移动达成
- viewport 外窗口不参与 render / hit-test / focus 候选
- query / IPC 能区分 scene 坐标与 screen 坐标

### 验证

- `cargo check --workspace`
- 重点单测：
  - viewport projection
  - floating move/resize on scene coordinates
  - focus / hit-test with viewport clipping
- 重点集成测试：
  - IPC viewport command roundtrip
  - virtual output capture reflects viewport movement

### 需要新增的测试

- ECS / shell 单测
  - scene 坐标到 presentation 坐标的投影边界：
    - 完全可见
    - 部分相交
    - 完全在 viewport 外
    - scene 坐标接近 `isize` 极值时不溢出
  - floating move / resize / interactive grab 更新 scene 坐标，而不是直接写 screen 坐标
  - viewport 平移后，窗口 scene 坐标不变，但 `SurfaceGeometry` 改变
- IPC / query 测试
  - output snapshot 包含 viewport origin
  - window snapshot 区分 `scene_x/scene_y` 和 `screen_x/screen_y`
  - viewport 控制命令可以正确入队并生效
- render / backend 测试
  - virtual output capture 反映 viewport 平移后的 screen 几何
  - viewport 外窗口不会出现在 render order / capture frame 中

## Phase 2: Per-Output Active Workspace

对应设计稿：

- [`agent/per_output_active_workspace_design.md`](/home/misaka/Code/nekoland/agent/per_output_active_workspace_design.md)

### 目标

- 去掉全局单 active workspace 假设
- 让每个 output 选择自己当前显示的 workspace
- 把 viewport 绑定到 output 当前显示的 workspace 语义上

### 子阶段

#### 2A. Output -> Workspace 关系建模

- 移除 `Workspace.active` 的主语义地位
- 新增 output 当前 workspace 关系
- 引入 focused output 状态，作为 workspace switch / 新窗口落点 / viewport action 的默认目标

#### 2B. Per-Output WorkArea 与焦点路由

- 把全局 `WorkArea` 逐步拆成 per-output work area
- workspace switch 默认作用于 focused output
- 新窗口默认挂到 focused output 的 current workspace
- pointer / keyboard focus fallback 改为按 output 路由

#### 2C. 清理全局 active 假设

- 逐步淘汰 `ActiveWorkspace`
- 移除“非 active workspace 整棵 `Disabled`”这类全局单例推断
- 更新 IPC/query snapshot，让 output 成为 workspace 可见性的主语

### 本阶段不做

- 不支持一个 workspace 同时镜像到多个 output
- 不做复杂的 output 排布 UI
- 不在这阶段引入 background role

### 退出条件

- 每个 output 都有自己的 current workspace
- viewport action 默认作用于 focused output 或显式选定 output
- 新窗口创建、workspace 切换、focus 路由不再依赖全局 active workspace
- query / IPC 能回答“某个 output 当前显示哪个 workspace”

### 验证

- `cargo check --workspace`
- 重点单测：
  - output current workspace mapping
  - focused output fallback rules
  - per-output work area updates
- 重点集成测试：
  - multi-output workspace switch
  - new window placement on focused output
  - focus routing across outputs

### 需要新增的测试

- workspace / output 路由单测
  - output 切换 workspace 时只影响目标 output
  - focused output 缺失时正确 fallback 到 primary output
  - workspace 不会被同时绑定到多个 output
- shell / lifecycle 集成测试
  - 新窗口默认进入 focused output 的 current workspace
  - pointer focus 与 keyboard fallback 只在目标 output 当前 workspace 内工作
  - per-output work area 变化不会污染其它 output
- IPC / query 测试
  - output snapshot 暴露 current workspace
  - workspace 查询不再依赖全局 active 字段解释可见性

## Phase 3: Output Background Window

对应设计稿：

- [`agent/output_background_design.md`](/home/misaka/Code/nekoland/agent/output_background_design.md)

### 目标

- 支持把窗口设置为某个 output 的背景
- 背景窗口只在目标 output 上渲染
- 背景窗口不参与普通 workspace scene 的 focus / stacking / hit-test

### 子阶段

#### 3A. Background Role 建模

- 为窗口增加 output background role 组件
- 保存角色切换前的 restore state
- 增加 control plane 动作：
  - `background_on(output)`
  - `clear_background()`

#### 3B. Output-Aware Render Filtering

- 扩展 render/present 输入，使 surface 可以携带目标 output
- winit / drm / virtual backend 都按目标 output 过滤
- background 只进入目标 output 的 render scene

#### 3C. 普通窗口路径排除

- layout 系统跳过 background role
- focus / hit-test / stacking 排除 background role
- frame composition 让 background 位于普通窗口场景之下，但仍属于 output-local layer

### 本阶段不做

- 不做 workspace-specific wallpaper
- 不做多背景层叠加策略
- 不做复杂背景动画

### 退出条件

- 背景窗口只渲染到目标 output
- 背景窗口不参与普通窗口焦点与堆叠
- 清除背景角色后窗口能恢复原始语义

### 验证

- `cargo check --workspace`
- 重点单测：
  - background role restore
  - output target filtering
  - focus exclusion
- 重点集成测试：
  - virtual output capture shows background only on target output
  - IPC / keybinding can set and clear background role

### 需要新增的测试

- role / restore 单测
  - 设置背景前后的 restore state 保存与恢复
  - background role 不进入普通 stacking / focus 候选
- render / backend 集成测试
  - 多 output 下背景只出现在目标 output
  - 清除 background role 后窗口重新回到普通 scene 渲染路径
- IPC / keybinding 测试
  - output background 的设置/清除命令可被 CLI 和 keybinding 驱动

## 总体测试计划

每个阶段都必须同时补三类测试，不能只做一种：

1. 单测
   - 落在 `nekoland-ecs`、`nekoland-shell`、`nekoland-render`、`nekoland-backend`
   - 覆盖纯数据变换、投影、控制面入队、状态恢复
2. 集成测试
   - 优先放在 `nekoland/tests/` 与 `tests/integration/`
   - 覆盖 IPC、focus、workspace/output 路由、virtual output capture
3. 回归测试
   - 每阶段都要补至少一个“旧行为不被打坏”的测试
   - 例如：
     - fullscreen / maximized 继续保持 output-local
     - layer-shell exclusive zone 不受 viewport scene 影响
     - popup 跟随 parent 的 presentation geometry

建议每阶段结束时至少跑：

- `cargo check --workspace`
- `cargo test -p nekoland-ecs -p nekoland-shell -p nekoland-render -p nekoland-backend`
- `cargo test -p nekoland --test ipc_control_plane`
- 与该阶段强相关的 in-process / integration 测试子集

## 跨阶段规则

- 新的用户可见控制动作优先通过 `PendingWindowControls` / `PendingWorkspaceControls` /
  `PendingOutputControls` 进入系统，不要回退到新的 transport-shaped request queue。
- 所有边界层字符串或裸数字都应尽快转成 typed selector / typed id / typed control。
- 不要把 viewport visibility 编码成 `WindowMode::Hidden`。
- output-local surface 与 workspace-scene surface 必须持续保持边界清晰：
  - layer-shell 是 output-local
  - output background 是 output-local role
  - 普通 workspace window 是 scene-space object，经 viewport 投影后才进入 output-local render

## 建议执行策略

实现时按下面顺序提交更稳：

1. 先做 Phase 1A 和最小 projection helper，确保 geometry 基础类型稳定。
2. 再做 Phase 1B/1C，把 viewport control 和消费方迁移收完。
3. 等 viewport foundation 通过测试后，再开始 Phase 2 的 output/workspace 关系替换。
4. 等 output routing 稳定后，最后加 Phase 3 的 background role。

不要把三阶段混在一个大 patch 里，否则很难定位 render / focus / lifecycle 回归来自哪一层。
