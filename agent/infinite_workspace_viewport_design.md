# Infinite Workspace Viewport 设计稿

Date: 2026-03-14

## 实施顺序

这是三阶段总计划里的 Phase 1，也是后续 workspace/output 重构的几何基础。

总计划入口见
[`agent/workspace_output_rollout_plan.md`](/home/misaka/Code/nekoland/agent/workspace_output_rollout_plan.md)。

## 背景

用户希望把 workspace 从“一个有固定大小的矩形”改成“无限平面”：

- 窗口的逻辑位置不再受 output 尺寸约束
- output 只提供一个固定大小的 viewport
- viewport 可以在 workspace 上平移
- viewport 的移动既可以由显式 action 驱动，也可以由“定位到某个窗口”一类动作驱动

当前实现与这个目标存在根本冲突：

- `SurfaceGeometry` 同时承担“窗口真实位置”和“屏幕呈现位置”
- `WorkArea` 是单个全局资源，语义仍然偏向“当前唯一可见桌面”
- focus / hit-test / render / damage 都直接消费 `SurfaceGeometry`
- output 只有尺寸，没有“自己正在看 workspace 的哪一块”这层状态

所以这次不能只改 layout；必须把“scene 坐标”和“presentation 坐标”拆开。

## 一句话模型

- workspace 是无限 scene，不再有固定宽高
- output 决定 viewport 的尺寸
- output 决定 viewport 当前在 scene 里的原点
- 窗口在 scene 里有稳定逻辑坐标
- render / focus / hit-test 只看 scene 经过 viewport 投影后的屏幕坐标

## 目标 Runtime Model

### 1. 坐标分层

建议引入新的 scene-space 类型：

```rust
pub type WorkspaceCoord = isize;

pub struct ScenePoint {
    pub x: WorkspaceCoord,
    pub y: WorkspaceCoord,
}

pub struct SceneRect {
    pub x: WorkspaceCoord,
    pub y: WorkspaceCoord,
    pub width: u32,
    pub height: u32,
}
```

语义：

- `SceneRect` 是窗口在无限 workspace 上的真实几何
- `SurfaceGeometry` 退化为 output-local 的 presentation rect
- scene -> presentation 的投影由 viewport 负责

这层拆分是必须的。只要 `SurfaceGeometry` 继续既表示“真实位置”又表示“屏幕位置”，就不可能安全支持 `isize::MIN..=isize::MAX` 范围的 workspace 坐标。

### 2. viewport 是 output-scoped 状态

用户描述的是“一个 output 大小的 viewport 在 workspace 上移动”，因此 viewport 应属于 output，而不是 workspace 的固定属性。

建议目标模型为：

```rust
pub struct OutputViewport {
    pub origin: ScenePoint,
}
```

viewport 的宽高不单独存，直接来自：

- `OutputProperties.width`
- `OutputProperties.height`

这样：

- 一个 output 的模式切换会自动改变 viewport 尺寸
- viewport 的“看哪里”由 `origin` 决定

## 与 workspace / output 关系的目标语义

viewport 本身是 output-scoped；它总是解释为：

- “这个 output 当前显示的 workspace 的哪一块”

因此它和 `per_output_active_workspace` 设计天然耦合。目标关系应该是：

- output 选择当前显示哪个 workspace
- output 同时持有该 workspace 的 viewport origin

这也是为什么不建议把 viewport 永久挂到 `Workspace` 上：

- 那会把 viewport 锁死成“每个 workspace 只有一个相机”
- 后续一旦需要 multi-output，每个 output 看同一个 workspace 的不同位置就会冲突

## 窗口几何的目标语义

### Floating / 手动定位窗口

这是本次需求的核心，也是第一阶段必须完成的部分。

建议把 `WindowPlacement` 的位置语义改成 scene-space：

```rust
pub struct WindowPosition {
    pub x: WorkspaceCoord,
    pub y: WorkspaceCoord,
}
```

对应规则：

- 控制面里的 `move_to` 写的是 scene 坐标
- 交互式拖动修改的是 scene 坐标
- viewport 平移不会改窗口 scene 坐标
- 窗口是否可见取决于 scene rect 是否与 viewport 相交

### Tiled / Maximized / Fullscreen

这三类模式当前仍然强依赖 output / work area 语义，不适合和 floating 一起一次性重写成“任意 scene 对象”。

因此迁移建议明确分阶段：

- Phase 1：viewport scene 先覆盖 floating / 手动定位路径
- Phase 2：再决定 tiled 是否也要成为 scene 上的稳定对象
- fullscreen / maximized 继续保持 output-local 语义，不跟随 viewport 平移

这条边界是刻意的。否则会把“无限 workspace camera”与“重做整个 tiling/fullscreen 语义”绑成一次高风险迁移。

## 投影模型

建议新增一个明确的 projection pass：

```text
scene geometry + output viewport -> presentation geometry
```

规则：

1. 先取窗口 scene rect
2. 减去 viewport origin
3. 得到 output-local 的屏幕坐标
4. 如果与 viewport 完全不相交，则从 render / hit-test / focus 候选中排除

注意两点：

- 不要靠改 `WindowMode::Hidden` 表达“当前不在 viewport 内”
- 不要把 viewport 裁剪语义混进 stacking 或 layout

viewport 外不可见只是 presentation 结果，不是窗口生命周期状态。

## Control Plane 设计

由于 viewport 是 output-scoped，目标控制面应优先落在 `OutputOps` / `PendingOutputControls`：

- `pan_viewport_by(dx, dy)`
- `move_viewport_to(x, y)`
- `center_viewport_on_window(surface_id)`

推荐语义：

- keybinding 默认作用于 focused output
- 如果当前还没有 focused output 概念，Phase 1 暂时 fallback primary output
- IPC 应允许显式指定 output selector

不建议把 viewport 平移建模成 `WindowControl`：

- “移动窗口”和“移动相机”是两个不同对象的状态
- 把 camera side effect 藏到 window action 里会让控制面变得不可预测

## IPC / Query 设计

viewport 引入后，窗口的 `x/y` 会变得歧义：

- 它到底是 scene 坐标
- 还是当前 output 上的 presentation 坐标

因此 query/IPC 不应继续只暴露一组匿名 `x/y`。

建议目标 API：

- `WindowSnapshot`
  - `scene_x`
  - `scene_y`
  - `screen_x`
  - `screen_y`
  - `visible_on_output`
- `WorkspaceSnapshot`
  - 保留身份信息
  - 不持有全局 active 语义
- `OutputSnapshot`
  - 增加当前 workspace
  - 增加 viewport origin

边界类型建议：

- 内部 runtime：`isize`
- IPC / serialization：`i64`

理由：

- runtime 可以满足用户要的 machine-word scene 坐标范围
- IPC JSON 仍然保持跨平台稳定

## Render / Focus / Input 影响面

以下路径都必须改成消费 presentation geometry，而不是 scene geometry：

- `compose_frame_system`
- pointer hit-test
- keyboard focus fallback
- protocol pointer focus routing
- backend present
- damage tracking

其中：

- layer-shell surface 仍然是 output-local
- output background 仍然是 output-local
- popup 位置继续从 parent 的 presentation rect 派生

也就是说，viewport 只影响普通 workspace window scene，不影响 output UI 层。

## 实施阶段

### Phase 1: Scene / Presentation 拆分基础

- 引入 `WorkspaceCoord` / `ScenePoint` / `SceneRect`
- 给普通窗口增加 scene-space 几何来源
- 让 floating placement / move / resize 使用 scene 坐标
- 新增 viewport projection helper
- 保持现有单 active workspace 路径可运行

### Phase 2: Output Viewport Controls

- 扩展 `PendingOutputControls`
- 增加 viewport 平移 / 定位窗口动作
- keybinding 支持手动平移 viewport
- IPC 增加 viewport 相关命令和 query 字段

### Phase 3: Presentation Consumer Migration

- focus / hit-test / protocol pointer routing 改用 presentation 坐标
- render / damage / capture 改用 viewport 投影后的 surface 几何
- viewport 外窗口从可见候选中排除，但不改 mode

### Phase 4: Output <-> Workspace 最终对齐

- 落地 `per_output_active_workspace` 设计
- 让 viewport 绑定到 output 当前显示的 workspace
- 拆掉全局 `Workspace.active` / `ActiveWorkspace`
- 逐步淘汰全局 `WorkArea`

## 验收标准

- floating 窗口的逻辑位置可以安全落在远超 `i32` 的 scene 范围
- viewport 平移不会修改窗口 scene 坐标
- “定位到窗口”通过移动 viewport 达成，而不是偷偷移动窗口
- viewport 外窗口不参与 render / hit-test / focus 候选
- fullscreen / maximized / layer-shell / output background 继续保持 output-local 语义
- IPC / query 可以明确区分 scene 坐标与屏幕坐标

## 与现有设计稿的关系

- [`per_output_active_workspace_design.md`](/home/misaka/Code/nekoland/agent/per_output_active_workspace_design.md)
  仍然定义 output 如何选择当前 workspace，但其“普通窗口几何是 output-local”的结论应被本设计替换为“scene-space + viewport projection”。
- [`output_background_design.md`](/home/misaka/Code/nekoland/agent/output_background_design.md)
  仍然成立，但 output background 明确属于 output-local surface，不参与 workspace viewport 投影。
