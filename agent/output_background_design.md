# Output Background Window 设计稿

Date: 2026-03-14

## 2026-03-14 更新

新的 viewport 方案已经把普通 workspace window 的目标几何模型更新为
“scene-space + output viewport projection”，详见
[`agent/infinite_workspace_viewport_design.md`](/home/misaka/Code/nekoland/agent/infinite_workspace_viewport_design.md)。

这不会改变本稿的核心结论，反而把边界说得更清楚了：

- output background 明确属于 output-local surface
- 它不参与 workspace viewport 投影
- 它应在 output-aware render 过滤之后、普通 workspace scene 之前参与组合

因此 background role 仍然应该作为 output-scoped display role 单独建模，而不是塞回
workspace scene 语义里

## 实施顺序

这是三阶段总计划里的 Phase 3，放在最后实现，依赖前两阶段先把 output-aware routing 与
scene/output-local 边界稳定下来。

总计划入口见
[`agent/workspace_output_rollout_plan.md`](/home/misaka/Code/nekoland/agent/workspace_output_rollout_plan.md)。

## 背景

目前 `nekoland` 的窗口模型已经把以下概念拆开：

- `WindowLayout`: 基础几何策略
- `WindowMode`: 展示约束
- `WindowStackingState`: z-order
- `WindowPlacement`: floating placement intent
- `WorkspaceTilingState`: workspace-scoped tiling tree

用户希望“将一个窗口设置为 output 的背景”。这个需求不属于 `layout`、`mode` 或 `stacking` 本身，而是一个新的 **output-scoped display role**：

- 它不是普通 workspace window
- 它不应该参与 focus / hit-test / stacking
- 它应该绑定到某个 output
- 它应该渲染在该 output 的最底层

## 现状问题

现有实现里存在两个关键约束：

1. `RenderList` 是全局的，不区分 output
2. backend present 路径默认把同一份 `RenderList` 渲染到所有 output

这意味着如果只给窗口加一个 “background” 标记，而不补 output-aware 过滤链路，会出现：

- 背景窗口在所有 output 上镜像显示
- 多 output 语义错误

所以这个需求不能只靠 shell 层增加一个组件完成，必须同时补 output-aware render filtering。

## 目标

- 支持把任意一个窗口设置为某个 output 的背景
- 背景窗口只在目标 output 上渲染
- 背景窗口不参与普通窗口 stacking
- 背景窗口不参与 pointer focus / keyboard focus fallback
- 背景窗口占满目标 output 的可见区域
- 清除背景角色后，窗口恢复为原来的普通窗口语义

## 非目标

- 这轮不做 workspace-specific wallpaper
- 不做多背景层叠加策略
- 不做背景窗口的专用动画/转场
- 不做“随 primary output 自动迁移”的复杂策略

## 设计原则

一句话模型：

- `layout` 决定普通窗口的基础几何
- `mode` 决定普通窗口的展示约束
- `stacking` 决定普通窗口的前后关系
- `output background role` 决定窗口是否脱离普通窗口语义并绑定到某个 output

也就是说，背景窗口应该是一个独立 role，而不是：

- `WindowMode::Background`
- `WindowLayout::Background`
- 或者往 `WindowStackingState` 里塞一个特殊 z-index

## 目标 Runtime Model

建议新增以下 ECS 组件：

```rust
#[derive(Component, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputBackgroundWindow {
    pub output: OutputName,
    pub restore: WindowRestoreState,
}
```

语义：

- `output`: 目标 output 名称
- `restore`: 清除背景角色时恢复窗口的原始 `geometry + layout + mode`

这里直接使用 `OutputName`，不把 background role 建模成 `ChildOf(output)`：

- `ChildOf(workspace)` 当前已经承担 workspace membership
- 用 `ChildOf(output)` 会混淆 parent 语义
- output background 是一个独立 role，不应覆盖 workspace relationship 的含义

## Render Model

### 1. RenderSurfaceSnapshot 增加目标 output

现有 backend present 通过：

- `RenderList`
- `HashMap<u64, RenderSurfaceSnapshot>`

拿到每个 surface 的几何和 role。

建议扩展：

```rust
pub struct RenderSurfaceSnapshot {
    pub geometry: SurfaceGeometry,
    pub role: RenderSurfaceRole,
    pub target_output: Option<OutputName>,
}
```

语义：

- `None`: 普通 surface，渲染到所有 output
- `Some(output)`: 只渲染到指定 output

这样可以避免改造 `RenderList` 结构本身，output-aware 过滤放到 backend present 路径完成。

### 2. backend present 过滤规则

在以下 backend 中加同一条过滤规则：

- `winit`
- `drm`
- `virtual_output`

规则：

```rust
if let Some(target_output) = surface.target_output.as_ref() {
    if target_output != &current_output.device.name {
        continue;
    }
}
```

这样背景窗口只会出现在目标 output。

## Shell / Render 行为

### 1. 新增 background layout system

建议新增：

```rust
pub fn output_background_layout_system(...)
```

职责：

- 查询所有 `OutputBackgroundWindow`
- 找到目标 output 的 `OutputProperties`
- 把窗口几何强制设为 output 全屏：
  - `x = 0`
  - `y = 0`
  - `width = output.width`
  - `height = output.height`

注意：由于当前 output 没有全局排布坐标，这个几何语义默认就是 output-local full rect。

### 2. 普通 layout 系统跳过背景窗口

以下系统都应跳过 `OutputBackgroundWindow`：

- `tiling_layout_system`
- `floating_layout_system`
- `fullscreen_layout_system`
- `stacking_layout_system`

原因：

- 背景窗口不属于普通 workspace window scene
- 其 geometry 由 output background system 接管
- 其前后关系不应由窗口 stack 决定

### 3. compose_frame_system 单独处理背景窗口

当前顺序：

1. background/bottom layer-shell
2. visible windows
3. popups
4. top/overlay layer-shell

建议改为：

1. output background windows
2. background/bottom layer-shell
3. visible normal windows
4. popups
5. top/overlay layer-shell

原因：

- output background 是 wallpaper 语义，应位于最底层
- layer-shell background/bottom 仍然可以覆盖在 wallpaper 之上

## Focus / Input 规则

背景窗口必须从普通 focus 候选中排除：

- `pointer_button_focus_system`
- `focus_management_system`
- 任何 keyboard focus fallback

规则：

- 背景窗口不可被点击聚焦
- 背景窗口不可成为 fallback focused surface
- 背景窗口不参与 raise/lower

如果用户确实要操作它，应该通过显式 control plane 指令，而不是普通点击。

## Control Plane

建议沿用现有 `PendingWindowControls`，不要新开一套背景专用队列。

新增：

```rust
pub enum WindowBackgroundControl {
    Set(OutputName),
    Clear,
}

pub struct PendingWindowControl {
    ...
    pub background: Option<WindowBackgroundControl>,
}
```

高层 API：

```rust
window.background_on(OutputName::from("eDP-1"));
window.clear_background();
```

理由：

- 这是“选中一个窗口，然后给它一个角色”的窗口控制动作
- 不需要单独引入 `PendingOutputBackgroundControls`
- 保持 control plane 一致性

## 生命周期语义

### 设置为背景

当执行 `background_on(output)`：

1. 若窗口当前还不是背景窗口：
   - 保存 `WindowRestoreState`
2. 插入/更新 `OutputBackgroundWindow`
3. 从普通窗口路径里排除：
   - focus
   - stacking
   - 普通 layout

### 清除背景

当执行 `clear_background()`：

1. 读取 `OutputBackgroundWindow.restore`
2. 恢复：
   - `geometry`
   - `layout`
   - `mode`
3. 移除 `OutputBackgroundWindow`

### 窗口销毁

背景窗口销毁时不需要特殊全局清理，只需正常 despawn。

## IPC / CLI

建议扩展 `WindowCommand`：

```rust
pub enum WindowCommand {
    ...
    Background { surface_id: u64, output: String },
    ClearBackground { surface_id: u64 },
}
```

CLI:

```bash
nekoland-msg window background 42 eDP-1
nekoland-msg window clear-background 42
```

边界层依然把 `String` 解析成 `OutputName`，进入 ECS 后不再传播裸字符串。

## Query / Snapshot

建议在 `WindowSnapshot` 中增加可选字段：

```rust
pub struct WindowSnapshot {
    ...
    pub background_output: Option<String>,
}
```

这样：

- IPC tree/query 可见
- 测试可直接验证
- 用户能知道某个窗口当前是不是 output 背景

## 测试建议

### ECS / shell unit tests

- 设置背景后，窗口从 stacking/focus 可见集排除
- 设置背景后，几何变成目标 output full rect
- 清除背景后，恢复原始 geometry/layout/mode
- compose order 中背景窗口位于最底层

### backend / render tests

- background window 只出现在目标 output 的 render 结果里
- 非目标 output 不应包含该 surface

### IPC / integration tests

- `window background` 指令生效
- `window clear-background` 指令恢复
- `query tree` 能看到 `background_output`

## 迁移顺序

1. 新增 `OutputBackgroundWindow`
2. 扩展 `RenderSurfaceSnapshot.target_output`
3. backend present 路径按 output 过滤 surface
4. shell 新增 `output_background_layout_system`
5. focus / stacking / normal layout 跳过背景窗口
6. 扩展 `PendingWindowControls` 与 `WindowControlApi`
7. 接 IPC / CLI / tests

## 风险

### 1. 多 output 目前没有全局坐标

现有系统默认每个 output 在自己的局部空间渲染，所以背景窗口的 geometry 只能表达
“填满该 output”，不能表达跨 output 大画布。

这对 wallpaper 语义是可接受的，但要明确不是“桌面大画布”模型。

### 2. RenderList 仍然是全局

本设计不重写 `RenderList`，而是把 output-aware 过滤放到 backend present。
这能最小化改动，但意味着：

- frame callback / damage 仍然更偏全局
- 多 output 下可能出现比严格最优更宽松的 callback/damage 行为

这是可接受的第一步。

## 开放问题

1. `background_on(primary)` 是否需要一等语义，跟随 primary output 切换？
2. 背景窗口是否应该默认禁用 server decorations？
3. 是否允许多个背景窗口绑定同一 output？
   当前建议：后设置者覆盖前者，或者明确只允许一个。

## 结论

“窗口作为 output 背景”应该实现为：

- 一个独立的 output-scoped window role
- 一个 output-aware backend render filter
- 一个最小的窗口控制动作 `background_on / clear_background`

而不应该把它塞进：

- `WindowLayout`
- `WindowMode`
- `WindowStackingState`

这套设计和当前已经完成的 layout/mode/stacking 分离方向是一致的，后续也能自然扩展到：

- wallpaper persistence
- per-output desktop scene
- output-scoped special surfaces
