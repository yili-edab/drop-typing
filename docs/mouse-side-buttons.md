# 鼠标侧键语音触发方案

> 状态：设计完成，待实施

## 目标

支持使用鼠标侧键触发语音录入和修复，作为键盘快捷键的补充方案。

## 按键分配

| 鼠标按键 | 通道 | 功能 |
|---------|------|------|
| 前进键（Button 5 / X2） | trigger | 长按录音、短按提交 |
| 后退键（Button 4 / X1） | repair | 长按说修正指令 |

不占用中键（系统高频功能，冲突过大），不绑鼠标 command 通道（物理按键不足）。

## 配置设计

拆分为键盘和鼠标两个独立配置段：

```toml
[hotkey.keyboard]
trigger = ["MetaRight"]          # macOS；Windows 为 ["Meta", "Alt"]
repair  = ["AltGr"]              # macOS；Windows 为 ["Control", "Alt"]
command = ["ShiftRight"]         # macOS；Windows 为 ["Meta", "Shift"]
cancel  = ["Escape"]

[hotkey.mouse]
trigger = "forward"               # 前进键（X2，长按录音、短按提交）
repair  = "back"                  # 后退键（X1，长按说修正指令）
# command 和 cancel 不绑鼠标
```

设计决策：

- `[hotkey.keyboard]` 保持现有行为不变（macOS 单键 OR / Windows 组合 AND）
- `[hotkey.mouse]` 全局统一语义：按住即触发，简单单键，无平台差异
- 鼠标段可整体缺省，不写 = 纯键盘模式
- 两段独立解析、互不干扰

## 涉及改动

### 1. vendored rdev 补丁

**文件**：`src-tauri/vendor/rdev/src/rdev.rs`

- `Button` 枚举增加 `Back`、`Forward` 变体

**文件**：`src-tauri/vendor/rdev/src/macos/common.rs`

- 映射 `CGEventType::OtherMouseDown` / `OtherMouseUp`
- 通过 `cg_event.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER)` 区分：
  - `kCGMouseButtonCenter` (2) → `Button::Middle`（暂不使用，但一并支持）
  - `kCGMouseButtonBackward` (3) → `Button::Back`（后退键）
  - `kCGMouseButtonForward` (4) → `Button::Forward`（前进键）

**文件**：`src-tauri/vendor/rdev/src/windows/common.rs`（如有 Windows 后端对应映射）

- 同样映射 `WM_XBUTTONDOWN` / `WM_XBUTTONUP` → `Back` / `Forward`
- 若当前无对应映射，需新增

### 2. 配置解析

**文件**：`src-tauri/src/config.rs`

- `HotkeyConfig` 新增 `mouse` 字段（`Option<MouseHotkeyConfig>`）
- `MouseHotkeyConfig` 包含 `trigger: Option<MouseButton>`、`repair: Option<MouseButton>`
- `MouseButton` 枚举：`Forward`（前进键/X2）、`Back`（后退键/X1）——用户配置中写作 `"forward"` / `"back"`
- 缺省时不绑定鼠标

**文件**：`src-tauri/src/hotkey/mod.rs`

- `Bindings` 新增 `mouse: MouseBindings` 字段
- `MouseBindings` 包含 `trigger: Option<MouseButton>`、`repair: Option<MouseButton>`
- `MouseButton` 为 rdev `Button` 的外部表示（`Back` / `Forward`），与键盘 `KeySpec` 独立——鼠标按键不混入键盘列表

### 3. macOS 热键监听

**文件**：`src-tauri/src/hotkey/macos.rs`

- `rdev::listen` 回调增加 `ButtonPress` / `ButtonRelease` 分支：
  - `Button::Back` → 匹配 `mouse.repair` → `RepairDown` / `RepairUp`
  - `Button::Forward` → 匹配 `mouse.trigger` → `TriggerDown` / `TriggerUp`
- 鼠标按键不参与 `OtherKeyDown` 逻辑——只有键盘修饰键才需要 taint 判定

### 4. Windows 热键监听

**文件**：`src-tauri/src/hotkey/windows.rs`

- `rdev::listen` 回调增加 `ButtonPress` / `ButtonRelease` 分支
- 鼠标按键**绕过修饰键状态机**，直接发对应事件
- 与 Esc 取消键同模式：单键独立触发，不参与组合逻辑

### 5. 流水线层（pipeline.rs）

**文件**：`src-tauri/src/pipeline.rs`

- 理论上**无需改动**——pipeline 只消费 `HotkeyEvent`，不关心来源是键盘还是鼠标
- 需确认：鼠标侧键的 `OtherKeyDown` 逻辑不同于键盘（鼠标按键不应 taint 键盘录音，反之亦然）
  - 当前录音期间任意 `OtherKeyDown` 都会 taint，需区分来源
  - 方案：`OtherKeyDown` 拆分为 `KeyboardInterrupt`（来自键盘）和忽略鼠标无关按键

### 6. 鼠标双击检测保持现状

- 左键双击（`MouseDoubleClick`）继续用于异常态消除和暂存条提交
- 侧键不做双击语义

## 不需要改动的地方

- `pipeline.rs` 状态机逻辑（长短按判定、ASR 流程、清洗流程）
- `staging.rs`（窗口管理）
- `inject/` 粘贴注入层
- `audio/` 录音层
- 前端（暂存条 UI）

## 验证方式

1. macOS：插上有侧键的鼠标，配置 `[hotkey.mouse]` 段，按住前进键 → 看到暂存条弹出 → 说话 → 松手 → ASR 结果追加
2. macOS：按住后退键 → 说修正指令 → 暂存条内容被修正
3. macOS：不配置 `[hotkey.mouse]` → 侧键行为不受影响，系统正常响应
4. macOS：侧键长按期间按键盘 → 不影响（不 taint）
5. macOS：键盘录音期间按侧键 → 不影响（不 taint）
6. Windows：同上逐项验证
7. 普通鼠标（无侧键）：不受任何影响

## 后续可扩展

- 若日后有更多按键的鼠标（如游戏鼠标的 Button 6-9），按同样模式扩展 `MouseBindings`
- 若需要鼠标 command 通道，加一个 `command: Option<u8>` 即可
