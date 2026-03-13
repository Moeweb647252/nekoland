# Per-Output Active Workspace 设计稿

Date: 2026-03-14

## 2026-03-14 更新

这份设计稿仍然描述“output 如何选择当前 workspace”的关系模型，但几何模型有一个重要更新：

- 普通窗口的目标语义不再是“纯 output-local 坐标”
- 新的几何主线改为“workspace scene 坐标 + output viewport 投影”
- 详细说明见
  [`agent/infinite_workspace_viewport_design.md`](/home/misaka/Code/nekoland/agent/infinite_workspace_viewport_design.md)

因此阅读本稿时，需要把下面几处旧表述替换成新的理解：

- `SurfaceGeometry` 不再应该继续兼任窗口真实位置与输出呈现位置
- output-local 只是 presentation 结果，不是普通窗口的唯一权威几何
- output 侧除了 current workspace，还需要 viewport origin 这层状态

## 实施顺序

这是三阶段总计划里的 Phase 2，依赖 Phase 1 的 viewport / projection foundation 先稳定。

总计划入口见
[`agent/workspace_output_rollout_plan.md`](/home/misaka/Code/nekoland/agent/workspace_output_rollout_plan.md)。

## 背景

当前 `nekoland` 的 workspace/runtime 语义仍然是全局单 active 模型：

- `Workspace.active: bool`
- `ActiveWorkspace` marker
- 非 active workspace 根会被整棵 `Disabled`
- `WorkArea` 是单个全局资源
- 普通窗口布局、focus、frame-callback、render 主要都围绕“当前唯一 active workspace”展开

这和“每个 output 都对应一个 Active Workspace”是冲突的。

用户想要的真实语义应该是：

- 每个 output 都有自己当前显示的 workspace
- workspace 切换默认作用在某个 output 上，而不是全局
- 多个 output 可以同时显示不同 workspace

## 现状问题

当前代码已经有两部分是多 output 友好的：

1. output 是 ECS 一等实体
2. layer-shell 已经支持 output targeting

但普通窗口 scene 还没有完成 per-output 化，主要问题有：

1. `Workspace.active` 是全局单例语义
2. `ActiveWorkspace` 只有一个
3. `WorkArea` 是全局单份，而不是 per-output
4. render scene 是全局 `RenderList`
5. 普通窗口没有 output-scoped visibility routing

## 关键约束

这件事有一个必须先说清的约束：

**在当前“一个窗口实体只有一份 `SurfaceGeometry`”的模型下，一个 workspace 不能同时显示在多个 output 上。**

否则会立即出现冲突：

- 同一个 workspace 上的同一个窗口
- 在不同 output 上需要不同几何
- 但 ECS 里只有一份 `SurfaceGeometry`

所以本设计明确采用下面的不变量：

- 每个 output 恰好对应一个当前 workspace
- 每个 workspace 在同一时刻至多绑定到一个 output

这是 per-output active workspace 的最小正确模型。

## 目标

- 去掉全局单 active workspace 假设
- 每个 output 都有自己的 current workspace
- 普通窗口仍然 `ChildOf(workspace)`，不直接变成 `ChildOf(output)`
- shell/layout/focus/render 都按 output -> workspace 映射工作
- 普通窗口的目标几何语义变成“workspace scene 坐标 + output viewport projection”

## 非目标

- 这轮不支持一个 workspace 同时镜像到多个 output
- 不做动态 output 排布 UI
- 不做跨 output 大画布桌面语义
- 不做 workspace 在 output 间的复杂动画切换

## 设计原则

一句话模型：

- output 决定“当前显示哪个 workspace”
- workspace 决定“有哪些窗口”
- layout 决定“这些窗口在该 output 上怎么摆”
- render 决定“这些窗口只渲染到对应 output”

也就是说，active workspace 不再是 workspace 自身的全局属性，而是 output 的状态。

## 目标 Runtime Model

### 1. 移除全局 active workspace 语义

建议逐步废弃：

- `Workspace.active`
- `ActiveWorkspace`

workspace 最终只保留身份元数据：

```rust
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
}
```

### 2. output 持有当前 workspace

建议新增 relationship：

```rust
#[derive(Component)]
pub struct OutputCurrentWorkspace(#[relationship] pub Entity);

#[relationship_target(relationship = OutputCurrentWorkspace)]
pub struct WorkspaceOnOutputs(Vec<Entity>);
```

语义：

- output entity 挂 `OutputCurrentWorkspace(workspace_entity)`
- workspace 可反查当前挂在哪个 output 上

这样：

- workspace 可见性来自 output 关系
- 不需要再把“active”复制进 workspace 元数据

### 3. focused output 单独建模

workspace shown on output 和 input focus 不是一回事。

建议新增：

```rust
#[derive(Resource)]
pub struct FocusedOutputState {
    pub name: Option<String>,
}
```

语义：

- 哪个 output 当前被 seat/pointer/keyboard 关注
- workspace switch / new window placement 默认作用于 focused output

## Output 几何基础

当前 output 只有：

- `OutputDevice`
- `OutputProperties { width, height, refresh, scale }`

但没有 output 在全局桌面中的位置。

如果要正确做：

- pointer 落在哪个 output
- 多 output focus routing
- output-aware fullscreen/maximize

就需要新增 output placement：

```rust
#[derive(Component)]
pub struct OutputPlacement {
    pub x: i32,
    pub y: i32,
}
```

语义：

- output 在 compositor 全局空间中的原点
- output 负责把 scene-space 窗口投影成 output-local 的 `SurfaceGeometry`
- pointer hit-test 和 output routing 通过 `OutputPlacement + OutputProperties` 求解

## Work Area

全局 `WorkArea` 必须拆成 per-output。

建议新增：

```rust
#[derive(Component)]
pub struct OutputWorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
```

语义：

- 每个 output 自己一份可用布局区域
- layer-shell exclusive zone 只影响自己目标 output 的 work area

完成后应删除全局 `WorkArea` 资源。

## 窗口归属与可见性

窗口继续保持：

```rust
ChildOf(workspace)
```

不把普通窗口直接挂到 output。

窗口对某个 output 是否可见，由下面两步决定：

1. 找到窗口所属 workspace
2. 看这个 workspace 当前是否是某个 output 的 `OutputCurrentWorkspace`

因为本设计要求“一个 workspace 至多属于一个 output”，所以窗口对 output 的可见性是唯一的。

## Shell 行为

### 1. workspace housekeeping

现有：

- 保证有一个全局 active workspace

目标：

- 保证每个 output 都有一个 `OutputCurrentWorkspace`
- 保证每个 workspace 最多被一个 output 引用
- 当 output 消失时，相关映射被移除或收敛

### 2. workspace switch

当前 `workspace switch 2` 是全局动作。

目标应变成：

```text
workspace switch 2 on focused-output
```

控制面默认规则：

- keybinding：默认切 focused output
- IPC/CLI：允许显式传 output selector；不传时默认 focused output

### 3. 新窗口创建

当前 XDG/X11 创建默认挂到 active workspace。

目标：

- 新窗口默认挂到 focused output 的 current workspace
- 如果没有 focused output，fallback primary output
- 如果还没有 output，再 fallback workspace 1

## Layout 行为

### 1. tiling

`WorkspaceTilingState` 已经是 per-workspace，这点可以保留。

但 layout 应改为：

- 先遍历 output
- 找到每个 output 当前显示的 workspace
- 用该 output 的 `OutputWorkArea` 与 viewport origin 给该 workspace 内窗口算 scene 几何或投影输入

因为 workspace 不会同时出现在多个 output，这不会和该阶段“每个窗口只有一份投影后
`SurfaceGeometry`”的实现约束冲突。

### 2. floating

floating placement 应使用窗口所属 output 的 `OutputWorkArea`，而不是全局 work area。

### 3. maximize / fullscreen

maximize / fullscreen 必须使用窗口所属 output 的尺寸或 work area，而不是 `outputs.iter().next()` 这种全局 fallback。

## Focus / Input

focus 路由要改成 output-aware：

1. 用 `OutputPlacement + OutputProperties` 找 pointer 当前在哪个 output
2. 取该 output 的 `OutputCurrentWorkspace`
3. 只在这个 workspace 的窗口里做 hit-test / fallback focus

keyboard focus fallback 也应按 focused output 处理，而不是从全局可见窗口里选 topmost。

## Render

### 1. 普通窗口需要 output 目标

普通窗口 scene 不再是“所有可见窗口一锅端”，而是每个窗口都要能解析出目标 output。

建议扩展 backend present 输入：

```rust
pub struct RenderSurfaceSnapshot {
    pub geometry: SurfaceGeometry,
    pub role: RenderSurfaceRole,
    pub target_output: Option<String>,
}
```

语义：

- `Some(output)`: 这个 surface 只渲染到该 output
- `None`: 对所有 output 可见的全局 surface

### 2. RenderList 是否需要拆分

这件事有两个方案：

#### 方案 A：保持全局 `RenderList`

- `RenderList` 仍然是全局排序
- 但 backend present 时按 `target_output` 过滤

优点：

- 改动小

缺点：

- frame callback / damage 仍然更偏全局

#### 方案 B：改成 per-output render plan

```rust
pub struct OutputRenderPlan {
    pub outputs: BTreeMap<String, Vec<RenderElement>>,
}
```

优点：

- render / damage / frame callback 全部按 output 精确建模

缺点：

- 改动更大

建议：

- 第一阶段用方案 A
- 第二阶段再决定是否升级成 per-output render plan

## Layer-Shell

layer-shell 已经有 output targeting，是当前最接近目标模型的部分。

需要做的不是重写它，而是把普通窗口对 output 的语义补齐到相同层级：

- layer 继续 `LayerOnOutput`
- 普通窗口通过 workspace -> output current mapping 获得目标 output

## Disabled / Visibility

现有做法是：

- 非 active workspace 整棵 `Disabled`

这在 per-output active workspace 下不成立，因为会有多个“当前可见”的 workspace。

建议改成：

- 任何当前被某个 output 引用的 workspace 都保持 enabled
- 不被任何 output 引用的 workspace 才 `Disabled`

这样仍然保留 `Disabled` 带来的查询优势，但不再依赖单例 active workspace。

## IPC / Query

现有 `WorkspaceSnapshot.active: bool` 不再足够。

建议改成：

```rust
pub struct WorkspaceSnapshot {
    pub id: u32,
    pub name: String,
    pub output: Option<String>,
}
```

或者：

```rust
pub struct WorkspaceSnapshot {
    pub id: u32,
    pub name: String,
    pub visible_on_output: Option<String>,
}
```

因为“active”已经不是全局布尔值。

## Control Plane

workspace 控制需要 output selector。

建议扩展：

```rust
pub enum WorkspaceControl {
    SwitchOrCreate { target: WorkspaceLookup, output: OutputSelector },
    Create { target: WorkspaceLookup },
    Destroy { target: WorkspaceSelector },
}
```

边界层：

- keybinding：默认 focused output
- IPC/CLI：支持显式 `--output`

## 迁移顺序

1. 新增 `OutputCurrentWorkspace`
2. 新增 `FocusedOutputState`
3. 新增 `OutputPlacement`
4. 新增 `OutputWorkArea`
5. workspace switch/create 新逻辑改成 output-scoped
6. 新窗口创建改成基于 focused output current workspace
7. shell layout 改为 per-output work area
8. focus / frame callback / render 改成 output-aware
9. 删除 `Workspace.active` / `ActiveWorkspace` / 全局 `WorkArea`

## 风险

### 1. 输出位置是缺失基础设施

如果没有 `OutputPlacement`，很多 per-output focus/hit-test 行为只能继续偷用：

- `outputs.iter().next()`
- 或 primary output fallback

这会让设计名义上是 per-output，实际运行时还是偏单 output。

### 2. 渲染链当前是全局 scene

如果只改 shell，不改 backend present filtering，多 output 行为会出错。

### 3. 测试面会比较广

受影响范围包括：

- shell workspace tests
- frame callback / presentation feedback tests
- ipc tree / workspace query tests
- config/runtime tests

## 结论

“每个 output 对应一个 Active Workspace” 应该落实为：

- output 持有 current workspace
- workspace 不再持有全局 active bool
- work area 改成 per-output
- layout/focus/render 都改成 output-aware

最关键的不变量是：

**一个 workspace 在同一时刻只能挂到一个 output。**

否则当前单窗口单几何模型无法成立。
