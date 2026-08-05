# Windows 兼容性修复实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 修复 drop-typing 在 Windows 上的兼容性问题，使用户从 Mac 拷贝配置后仍可正常使用：Win+Alt 录入不弹开始菜单、系统 Win 快捷键不受影响、精确左右修饰键可按配置触发、唤醒词「DT打」在安装包与裸 exe 下都能工作、script 动作可在 Windows 执行、失败不再静默。

**架构：** 在 Windows 热键引擎中把「左右合并的修饰键家族布尔状态」改为「按物理键记录按下集合 + 按 KeySpec 匹配」，让家族键与精确键共用同一套匹配逻辑；vendored rdev 的 Win 键拦截从「无条件吞」改为「仅吞 App 标记为属于 drop-typing 组合的 Win 键事件」；打包增加 `tauri.windows.conf.json` 产出 NSIS/MSI，工作流把 `models` 复制到裸 exe 旁并校验安装包存在；唤醒词加载/监听失败从 stderr 提升为暂存条黄底红字；`script.rs` 按平台选择 shell。

**技术栈：** Rust（rdev vendored patch、sherpa-onnx 静态链接）、Tauri 2、GitHub Actions（windows-latest）、Vite + TypeScript。

---

## 背景与已确认事实

- 用户在 Windows 上运行的是 GitHub Actions 产出的**裸 `drop-typing.exe`**（无 NSIS/MSI 安装包）。
- 拷贝的 `~/.drop-typing.toml` 带 `[hotkey]` 段（内容是 Mac 侧精确键：`MetaRight` / `AltGr` / `ShiftRight` 这类）。
- 现象：按住 Win+Alt **暂存条能出现**（说明当前配置的 trigger 实际可触发），但唤醒词「DT打」不响应。
- 用户要求：默认快捷键不分左右；用户显式配置精确左右键时必须只按指定侧才触发。
- 用户要求：开始菜单、Win+E 等系统快捷键保持可用；同时 Win+Alt 录音时不能弹开始菜单。
- 代码事实：
  - `src-tauri/src/hotkey/windows.rs:42` 的 `combo_from_specs` 会忽略所有 `KeySpec::Exact`（包括 `MetaRight`、`AltGr`、`ShiftRight`），组合变成空组合 → 永远不匹配。
  - `src-tauri/vendor/rdev/src/windows/listen.rs:51` 对 vk 0x5B/0x5C **无条件** `return 1`，App 运行期间开始菜单 / Win+E / Win+R 等全部失效。
  - `src-tauri/tauri.conf.json:21` `bundle.targets = "app"` 是 macOS 专属；`.github/workflows/build-windows.yml:53` 却找 NSIS/MSI，找不到只 warn，实际上只上传了裸 exe。
  - 裸 exe 旁没有 `models/builtin/...`，`wakeword::create_engine` 找不到模型只打 stderr，静默降级（`src-tauri/src/wakeword/sherpa.rs:32`、`src-tauri/src/pipeline.rs:159`）。
  - `src-tauri/src/script.rs:85` 一行命令写死 `/bin/zsh -lc`，Windows 必失败；测试模块用了 `std::os::unix::fs::PermissionsExt`，Windows 上 `cargo test` 编译不过。
  - `src/main.ts:15` 占位文案与 `settings.html:330` 帮助文案写死「右 ⌘」。

## 范围

包含：

1. Windows 热键引擎支持精确修饰键（区分左右）+ 组合匹配纯函数化以便测试。
2. vendored rdev 按需拦截 Win 键（保留系统 Win 快捷键）。
3. Windows 打包（NSIS/MSI）+ 裸 exe 携带模型 + 模型查找回退。
4. 唤醒词失败可见化（暂存条黄底红字）。
5. `script.rs` Windows 平台化（cmd.exe / .bat / .ps1）。
6. UI 文案、设置页提示、`config.example.toml` 文档更新。
7. CI（Windows 上跑 `cargo test`、校验安装包、上传 models）与 README/AGENTS.md 更新。

不包含（YAGNI，本次不做）：

- macOS 侧行为改动（caret 定位、双击确认等维持现状）。
- 唤醒词模型自动下载。
- 剪贴板非文本内容恢复（既有已知限制）。
- 把 Windows 默认 command 键从 Win+Shift 换掉（仅文档提示与 Win+Shift+S 的冲突，用户可自行在设置页改）。

## 文件结构

新增：

- `docs/superpowers/plans/2026-08-05-windows-compat-fixes.md`（本计划）
- `src-tauri/tauri.windows.conf.json`（Windows 平台 bundle targets）

修改：

- `src-tauri/src/hotkey/mod.rs`（平台无关组合匹配函数 + 测试）
- `src-tauri/src/hotkey/windows.rs`（精确键状态机 + Win 键吞放钩子）
- `src-tauri/vendor/rdev/src/windows/listen.rs`（按需拦截 Win 键）
- `src-tauri/vendor/rdev/src/lib.rs`（导出新 API）
- `src-tauri/src/wakeword/sherpa.rs`（exe 同目录模型查找 + 测试）
- `src-tauri/src/pipeline.rs`（唤醒词失败 → 暂存条错误）
- `src-tauri/src/script.rs`（平台化 + 测试分平台）
- `src/main.ts`、`settings.html`、`src/settings.ts`（文案）
- `config.example.toml`、`README.md`、`AGENTS.md`（文档）
- `.github/workflows/build-windows.yml`（models 复制、安装包校验、cargo test、上传 models）

## 执行前注意（脏工作区）

工作区已有用户未提交改动：`AGENTS.md`、`README.md`、`config.example.toml`、`src-tauri/src/config.rs`、`src-tauri/src/pipeline.rs`。**不得 reset / checkout / 覆盖**。

任务 4 会继续改 `pipeline.rs`，任务 6/7 会改 `config.example.toml`、`README.md`、`AGENTS.md`。执行开始前必须向用户确认：

- 方案 A（推荐）：先把用户现有 WIP 单独提交（如 `chore: 实时 ASR 转发器与倒计时默认值`），之后本计划的每个 commit 只含本任务文件。
- 方案 B：WIP 保持未提交，任务只 stage 自己改动的 hunk（`git add -p`），避免把用户 WIP 混入本计划 commit。

## 任务 1：平台无关组合匹配函数（hotkey/mod.rs）

**文件：** `src-tauri/src/hotkey/mod.rs`

把「一组 KeySpec 是否被当前按下的修饰键集合满足」等逻辑做成平台无关纯函数，便于在 macOS 上跑测试；Windows 引擎（任务 2）直接复用。

- [ ] **步骤 1：在 `hotkey/mod.rs` 的 `KeySpec` 实现之后添加函数**

```rust
/// 修饰键（Windows 组合只允许修饰键参与）。
pub(crate) fn is_modifier_key(key: &Key) -> bool {
    matches!(
        key,
        Key::ControlLeft
            | Key::ControlRight
            | Key::MetaLeft
            | Key::MetaRight
            | Key::Alt
            | Key::AltGr
            | Key::ShiftLeft
            | Key::ShiftRight
    )
}

/// 一组 KeySpec 是否全部被「当前按下的修饰键集合」满足。
/// 家族规格（Family）任意一侧即可；精确规格（Exact）必须该物理键按下。
pub(crate) fn specs_matched_by_pressed(
    specs: &[KeySpec],
    pressed: &std::collections::HashSet<Key>,
) -> bool {
    !specs.is_empty()
        && specs.iter().all(|s| match s {
            KeySpec::Family(f) => pressed.iter().any(|k| f.matches(k)),
            KeySpec::Exact(k) => pressed.contains(k),
        })
}

/// 某个键是否属于这组 KeySpec（组合结束 / taint 判定）。
pub(crate) fn key_matches_any_spec(key: &Key, specs: &[KeySpec]) -> bool {
    specs.iter().any(|s| s.matches(key))
}

/// 组合是否包含 Meta（Win）键——决定是否需要吞 Win 键事件。
pub(crate) fn specs_use_meta(specs: &[KeySpec]) -> bool {
    specs.iter().any(|s| match s {
        KeySpec::Family(ModFamily::Meta) => true,
        KeySpec::Exact(Key::MetaLeft | Key::MetaRight) => true,
        _ => false,
    })
}
```

- [ ] **步骤 2：在 `hotkey/mod.rs` 的 `mod tests` 中添加失败测试**

在现有 `#[cfg(test)] mod tests` 内追加：

```rust
    #[test]
    fn specs_matched_by_pressed_family_and_exact() {
        let mut pressed = std::collections::HashSet::new();
        pressed.insert(Key::MetaRight);
        pressed.insert(Key::AltGr);

        let family = vec![
            KeySpec::Family(ModFamily::Meta),
            KeySpec::Family(ModFamily::Alt),
        ];
        assert!(specs_matched_by_pressed(&family, &pressed));

        let exact = vec![
            KeySpec::Exact(Key::MetaRight),
            KeySpec::Exact(Key::AltGr),
        ];
        assert!(specs_matched_by_pressed(&exact, &pressed));

        let wrong_side = vec![
            KeySpec::Exact(Key::MetaLeft),
            KeySpec::Exact(Key::AltGr),
        ];
        assert!(!specs_matched_by_pressed(&wrong_side, &pressed));

        assert!(!specs_matched_by_pressed(&[], &pressed));
    }

    #[test]
    fn key_matches_any_spec_covers_family_and_exact() {
        let specs = vec![
            KeySpec::Exact(Key::MetaRight),
            KeySpec::Family(ModFamily::Alt),
        ];
        assert!(key_matches_any_spec(&Key::MetaRight, &specs));
        assert!(key_matches_any_spec(&Key::AltGr, &specs));
        assert!(!key_matches_any_spec(&Key::MetaLeft, &specs));
        assert!(!key_matches_any_spec(&Key::ShiftLeft, &specs));
    }

    #[test]
    fn specs_use_meta_detects_family_and_exact() {
        assert!(specs_use_meta(&[KeySpec::Family(ModFamily::Meta)]));
        assert!(specs_use_meta(&[KeySpec::Exact(Key::MetaRight)]));
        assert!(!specs_use_meta(&[KeySpec::Family(ModFamily::Alt)]));
    }

    #[test]
    fn modifier_key_classification() {
        assert!(is_modifier_key(&Key::ControlLeft));
        assert!(is_modifier_key(&Key::ControlRight));
        assert!(is_modifier_key(&Key::MetaLeft));
        assert!(is_modifier_key(&Key::MetaRight));
        assert!(is_modifier_key(&Key::Alt));
        assert!(is_modifier_key(&Key::AltGr));
        assert!(is_modifier_key(&Key::ShiftLeft));
        assert!(is_modifier_key(&Key::ShiftRight));
        assert!(!is_modifier_key(&Key::KeyA));
        assert!(!is_modifier_key(&Key::Escape));
    }
```

- [ ] **步骤 3：运行测试确认失败**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib hotkey::
```

预期：编译失败，报 `specs_matched_by_pressed` 等函数不存在。

- [ ] **步骤 4：实现函数（把步骤 1 的代码粘入）后重跑测试**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib hotkey::
```

预期：4 个新测试 PASS，原有 hotkey 测试保持 PASS。

- [ ] **步骤 5：Commit**

```bash
git add src-tauri/src/hotkey/mod.rs
git commit -m "test(hotkey): 组合匹配纯函数与精确键测试"
```

## 任务 2：Windows 热键引擎支持精确键 + Win 键按需拦截

**文件：**
- 修改：`src-tauri/src/hotkey/windows.rs`
- 修改：`src-tauri/vendor/rdev/src/windows/listen.rs`
- 修改：`src-tauri/vendor/rdev/src/lib.rs`

### 2a：vendored rdev 提供「按需吞 Win 键」API

- [ ] **步骤 1：在 `src-tauri/vendor/rdev/src/windows/listen.rs` 顶部添加原子标志与公开函数**

```rust
use std::sync::atomic::{AtomicBool, Ordering};

/// drop-typing：标记下一次 Win 键「按下」需要被吞掉（属于 drop-typing 组合）。
static SWALLOW_WIN_DOWN: AtomicBool = AtomicBool::new(false);
/// drop-typing：标记下一次 Win 键「松开」需要被吞掉（属于 drop-typing 组合）。
static SWALLOW_WIN_UP: AtomicBool = AtomicBool::new(false);

pub fn set_swallow_win_down(swallow: bool) {
    SWALLOW_WIN_DOWN.store(swallow, Ordering::SeqCst);
}

pub fn set_swallow_win_up(swallow: bool) {
    SWALLOW_WIN_UP.store(swallow, Ordering::SeqCst);
}
```

- [ ] **步骤 2：把 `raw_callback` 末尾的无条件拦截替换为按标志拦截**

把：

```rust
        // 拦截左右 Win 键：返回非零阻止消息传递到系统，避免弹出开始菜单。
        // 用户回调已在上方调用——App 仍能收到 MetaLeft/MetaRight 事件。
        if is_win {
            return 1;
        }
```

替换为：

```rust
        // drop-typing：只有 App 标记为「属于 drop-typing 组合」的 Win 键事件才拦截；
        // 其它 Win 键事件（开始菜单、Win+E、Win+R 等）放行给系统。
        if is_win {
            let down = matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN);
            let swallow = if down {
                SWALLOW_WIN_DOWN.swap(false, Ordering::SeqCst)
            } else {
                SWALLOW_WIN_UP.swap(false, Ordering::SeqCst)
            };
            if swallow {
                return 1;
            }
        }
```

注意：App 回调在 raw_callback 中先于本段执行（现有代码顺序），因此状态机在回调里设置标志、本段随即消费，顺序正确。

- [ ] **步骤 3：在 `src-tauri/vendor/rdev/src/lib.rs` 的 Windows 导出区添加公开导出**

在：

```rust
#[cfg(target_os = "windows")]
pub use crate::windows::Keyboard;
```

之后添加：

```rust
#[cfg(target_os = "windows")]
pub use crate::windows::{set_swallow_win_down, set_swallow_win_up};
```

### 2b：重写 windows.rs 状态机

- [ ] **步骤 4：替换 `ComboDef` / `combo_from_specs` / `combo_matches` / `mod_family` / `family_in_combo`**

把 `src-tauri/src/hotkey/windows.rs` 中从 `struct ComboDef` 到 `fn family_in_combo` 结尾的整体替换为：

```rust
/// 经配置解析后的组合通道定义
struct ComboDef {
    /// 激活时发送哪个 Down/Up 事件
    down: HotkeyEvent,
    up: HotkeyEvent,
    /// 需要哪些键（家族或精确修饰键）同时按下
    specs: Vec<KeySpec>,
}

/// 从用户配置的 KeySpec 列表构建组合定义。
///
/// Windows 组合只接受修饰键：家族名（Control/Meta/Alt/Shift）或精确修饰键名
/// （ControlLeft/ControlRight/MetaLeft/MetaRight/Alt/AltGr/ShiftLeft/ShiftRight）。
/// 其它精确键（如 KeyA）不适用，会被忽略并打印警告。
fn combo_from_specs(specs: &[KeySpec], down: HotkeyEvent, up: HotkeyEvent) -> ComboDef {
    let mut kept = Vec::new();
    for s in specs {
        match s {
            KeySpec::Family(_) => kept.push(s.clone()),
            KeySpec::Exact(key) if super::is_modifier_key(key) => kept.push(s.clone()),
            KeySpec::Exact(key) => {
                eprintln!(
                    "[drop-typing] 警告：Windows 组合键不支持非修饰键 {key:?}，已忽略。\
                     请使用 Control/Meta/Alt/Shift 家族名或精确修饰键名（含左右）。"
                );
            }
        }
    }
    ComboDef { down, up, specs: kept }
}

/// 检查当前按下的修饰键集合是否满足某个组合定义
fn combo_matches(def: &ComboDef, pressed: &HashSet<Key>) -> bool {
    super::specs_matched_by_pressed(&def.specs, pressed)
}

/// Win 键按下时，判断它是否即将补全某个已配置的 Meta 组合
/// （其它组合修饰键已按住，只差这个 Win 键）。
/// 用于在 keydown 阶段吞掉已知属于 drop-typing 组合的 Win 键，
/// 防止开始菜单在任何触发时机弹出。
fn win_completes_meta_combo(key: &Key, combos: &[ComboDef], pressed: &HashSet<Key>) -> bool {
    combos.iter().any(|c| {
        super::specs_use_meta(&c.specs)
            && c.specs.iter().all(|s| match s {
                KeySpec::Family(ModFamily::Meta) => true,
                KeySpec::Exact(Key::MetaLeft) => {
                    key == &Key::MetaLeft || pressed.contains(&Key::MetaLeft)
                }
                KeySpec::Exact(Key::MetaRight) => {
                    key == &Key::MetaRight || pressed.contains(&Key::MetaRight)
                }
                KeySpec::Family(f) => pressed.iter().any(|k| f.matches(k)),
                KeySpec::Exact(k) => pressed.contains(k),
            })
    })
}
```

- [ ] **步骤 5：替换 listen 闭包内的状态与 KeyPress / KeyRelease 分支**

把：

```rust
                // 修饰键状态（左右合并为同一家族）
                let (mut ctrl, mut win, mut alt, mut shift) =
                    (false, false, false, false);
                // 当前激活的组合在 combos 中的索引
                let mut active_idx: Option<usize> = None;
                // 鼠标左键双击检测：记录上一次左键按下时间
                let mut last_left_press: Option<std::time::Instant> = None;
```

替换为：

```rust
                // 当前按下的修饰键集合（区分左右）
                let mut pressed: HashSet<Key> = HashSet::new();
                // 已被 drop-typing 组合使用的 Win 键（松开时吞掉，避免开始菜单）
                let mut win_used: HashSet<Key> = HashSet::new();
                // 当前激活的组合在 combos 中的索引
                let mut active_idx: Option<usize> = None;
                // 鼠标左键双击检测：记录上一次左键按下时间
                let mut last_left_press: Option<std::time::Instant> = None;
```

再把 `EventType::KeyPress` 分支（从 `// ── 按键按下 ──` 到 `// 非修饰键 + 无组合激活 → 忽略`）替换为：

```rust
                        // ── 按键按下 ──
                        EventType::KeyPress(ref key) => {
                            // 清空键：无组合激活时单按即清空
                            if active_idx.is_none()
                                && cancel_specs.iter().any(|s| s.matches(key))
                            {
                                let _ = tx.send(HotkeyEvent::CancelDown);
                                return;
                            }

                            if super::is_modifier_key(key) {
                                // Win 键按下且即将补全某个 Meta 组合（如 Alt 已按住）：
                                // 吞掉本次 keydown，避免开始菜单弹出
                                if matches!(key, Key::MetaLeft | Key::MetaRight)
                                    && active_idx.is_none()
                                    && win_completes_meta_combo(key, &combos, &pressed)
                                {
                                    rdev::set_swallow_win_down(true);
                                }
                                pressed.insert(*key);
                                if active_idx.is_none() {
                                    // 检查是否有新的组合形成
                                    if let Some(idx) = combos.iter().position(|c| {
                                        combo_matches(c, &pressed)
                                    }) {
                                        active_idx = Some(idx);
                                        if super::specs_use_meta(&combos[idx].specs) {
                                            // 记录组合用到的 Win 键：松开时吞掉
                                            for k in pressed.iter().filter(|k| {
                                                matches!(k, Key::MetaLeft | Key::MetaRight)
                                            }) {
                                                win_used.insert(*k);
                                            }
                                            // 若本次按下正是 Win 键（Alt 先按住），吞掉 keydown
                                            if matches!(key, Key::MetaLeft | Key::MetaRight) {
                                                rdev::set_swallow_win_down(true);
                                            }
                                        }
                                        let _ = tx.send(combos[idx].down.clone());
                                    }
                                } else if !super::key_matches_any_spec(
                                    key,
                                    &combos[active_idx.unwrap()].specs,
                                ) {
                                    // 组合激活期间，按下不属于该组合的修饰键 → taint
                                    let _ = tx.send(HotkeyEvent::OtherKeyDown);
                                }
                            } else if active_idx.is_some() {
                                // 非修饰键 + 组合激活中 → taint
                                let _ = tx.send(HotkeyEvent::OtherKeyDown);
                            }
                            // 非修饰键 + 无组合激活 → 忽略
                        }
```

把 `EventType::KeyRelease` 分支（从 `// ── 按键释放 ──` 到 `// 非修饰键释放 → 忽略`）替换为：

```rust
                        // ── 按键释放 ──
                        EventType::KeyRelease(ref key) => {
                            if super::is_modifier_key(key) {
                                let was_active = active_idx;

                                // 如果释放的键属于当前激活组合 → 结束该组合
                                if let Some(idx) = active_idx {
                                    if super::key_matches_any_spec(key, &combos[idx].specs) {
                                        let _ = tx.send(combos[idx].up.clone());
                                        active_idx = None;
                                    }
                                }

                                // Win 键松开：属于已激活组合则吞掉这次 keyup
                                if matches!(key, Key::MetaLeft | Key::MetaRight)
                                    && win_used.remove(key)
                                {
                                    rdev::set_swallow_win_up(true);
                                }

                                pressed.remove(key);

                                // 如果刚结束了一个组合，检查是否新形成了另一个组合
                                // （「滑键」场景：保持 Win 不放，从 Ctrl 滑到 Alt）
                                if was_active.is_some() && active_idx.is_none() {
                                    if let Some(idx) = combos.iter().position(|c| {
                                        combo_matches(c, &pressed)
                                    }) {
                                        active_idx = Some(idx);
                                        if super::specs_use_meta(&combos[idx].specs) {
                                            for k in pressed.iter().filter(|k| {
                                                matches!(k, Key::MetaLeft | Key::MetaRight)
                                            }) {
                                                win_used.insert(*k);
                                            }
                                        }
                                        let _ = tx.send(combos[idx].down.clone());
                                    }
                                }
                            }
                            // 非修饰键释放 → 忽略
                        }
```

- [ ] **步骤 6：更新 `windows.rs` 顶部 import**

把：

```rust
use std::sync::mpsc;
```

替换为：

```rust
use std::collections::HashSet;
use std::sync::mpsc;
```

（`HashSet` 会在新状态机中使用。）

- [ ] **步骤 7：本地编译验证（macOS 侧不影响）**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

预期：macOS 编译通过（`windows.rs` 不参与 macOS 编译，但 `mod.rs` 改动已由任务 1 测试覆盖）。

- [ ] **步骤 8：Windows 编译验证（提交后由 CI 把关）**

先提交，再由任务 7 的 Windows workflow 编译；若 Windows 编译报错，回到步骤 4-6 修复。

- [ ] **步骤 9：Commit**

```bash
git add src-tauri/src/hotkey/windows.rs src-tauri/vendor/rdev/src/windows/listen.rs src-tauri/vendor/rdev/src/lib.rs
git commit -m "fix(hotkey): Windows 支持精确左右修饰键并按需拦截 Win 键"
```

## 任务 3：Windows 打包与模型资源

**文件：**
- 新增：`src-tauri/tauri.windows.conf.json`
- 修改：`.github/workflows/build-windows.yml`
- 修改：`src-tauri/src/wakeword/sherpa.rs`

### 3a：产出 NSIS/MSI 安装包

- [ ] **步骤 1：新增 `src-tauri/tauri.windows.conf.json`**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "bundle": {
    "targets": ["nsis", "msi"]
  }
}
```

说明：Tauri 会自动合并平台配置文件；macOS 仍使用主配置的 `"app"`，Windows 产出 NSIS + MSI，`bundle.resources`（模型）随之进入安装包。

### 3b：裸 exe 旁带上模型

- [ ] **步骤 2：在 workflow 的 Build 步骤之后、Upload 之前插入「复制模型到 exe 旁」与「校验安装包」**

在 `.github/workflows/build-windows.yml` 的 `Build Tauri app` 步骤后插入：

```yaml
      - name: Copy wakeword models next to raw exe
        shell: pwsh
        run: Copy-Item -Path src-tauri/models -Destination src-tauri/target/release/models -Recurse -Force

      - name: Verify Windows bundles exist
        shell: pwsh
        run: |
          $nsis = Get-ChildItem src-tauri/target/release/bundle/nsis/*.exe -ErrorAction SilentlyContinue
          $msi = Get-ChildItem src-tauri/target/release/bundle/msi/*.msi -ErrorAction SilentlyContinue
          if (-not $nsis) { throw "缺少 NSIS 安装包，请检查 tauri.windows.conf.json" }
          if (-not $msi) { throw "缺少 MSI 安装包，请检查 tauri.windows.conf.json" }
```

- [ ] **步骤 3：更新 Upload 与 Release 的文件列表**

把两处（Upload build artifacts、Create GitHub Release）的 path/files 各改为：

```yaml
            src-tauri/target/release/bundle/nsis/*.exe
            src-tauri/target/release/bundle/msi/*.msi
            src-tauri/target/release/drop-typing.exe
            src-tauri/target/release/models/**/*.onnx
            src-tauri/target/release/models/**/*.txt
            src-tauri/target/release/models/**/*.json
```

并把 `if-no-files-found: warn` 改为 `if-no-files-found: error`。

### 3c：模型查找增加 exe 同目录回退

- [ ] **步骤 4：在 `src-tauri/src/wakeword/sherpa.rs` 添加查找辅助函数**

在 `resolve_model_dir` 之前添加：

```rust
/// 在多个基目录下依次查找 `models/builtin/{model_dir}`。
fn find_in_dirs(model_dir: &str, bases: &[&Path]) -> Option<PathBuf> {
    bases
        .iter()
        .map(|base| base.join("models").join("builtin").join(model_dir))
        .find(|p| p.is_dir())
}
```

- [ ] **步骤 5：在 `resolve_model_dir` 中插入 exe 同目录回退**

在 `// 回退查找：尝试 CARGO_MANIFEST_DIR 下的 models/builtin` 之前插入：

```rust
    // 回退：可执行文件旁的 models/builtin（Windows 裸 exe / 便携部署）
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            eprintln!(
                "[drop-typing] 唤醒词：尝试 exe 同目录 '{}'",
                exe_dir.display(),
            );
            if let Some(p) = find_in_dirs(model_dir, &[exe_dir]) {
                return Some(p);
            }
        }
    }
```

（`resource_dir` 分支仍保持最优先；`CARGO_MANIFEST_DIR`、相对路径分支不变。）

- [ ] **步骤 6：在 `sherpa.rs` 现有 `#[cfg(test)] mod tests` 内追加测试**

```rust
    #[test]
    fn find_in_dirs_locates_model_dir() {
        let root = std::env::temp_dir().join(format!(
            "drop-typing-sherpa-test-{}",
            std::process::id()
        ));
        let model = root.join("models").join("builtin").join("m1");
        std::fs::create_dir_all(&model).unwrap();

        let result = find_in_dirs("m1", &[&root]);
        assert_eq!(result.as_deref(), Some(model.as_path()));
        assert_eq!(find_in_dirs("missing", &[&root]), None);

        let _ = std::fs::remove_dir_all(&root);
    }
```

- [ ] **步骤 7：运行测试**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib wakeword::
```

预期：新测试 PASS。

- [ ] **步骤 8：Commit**

```bash
git add src-tauri/tauri.windows.conf.json src-tauri/src/wakeword/sherpa.rs .github/workflows/build-windows.yml
git commit -m "build(windows): 产出 NSIS/MSI 并让裸 exe 携带唤醒词模型"
```

## 任务 4：唤醒词失败可见化

**文件：** `src-tauri/src/pipeline.rs`（该文件已有用户 WIP，按「执行前注意」处理）

- [ ] **步骤 1：定义 `WakeOutcome` 并替换通道类型**

在 `src-tauri/src/pipeline.rs` 的 `enum WakeCommand` 附近添加：

```rust
/// 唤醒词管理线程 → pipeline 的更新结果。
enum WakeOutcome {
    /// 引擎与监听均已就绪
    Ready(WakewordConfig, Arc<RingBuffer>, mpsc::Receiver<WakeEvent>),
    /// 监听/引擎失败，附用户可理解的错误信息
    Failed(String),
    /// 已关闭
    Disabled,
}
```

把 `wake_manager_loop` 的 `out_tx` 参数类型从：

```rust
    out_tx: mpsc::Sender<Option<(WakewordConfig, Arc<RingBuffer>, mpsc::Receiver<WakeEvent>)>>,
```

改为：

```rust
    out_tx: mpsc::Sender<WakeOutcome>,
```

把 `start()` 中的 channel 声明：

```rust
    let (wake_out_tx, wake_out_rx) = mpsc::channel::<
        Option<(WakewordConfig, Arc<RingBuffer>, mpsc::Receiver<WakeEvent>)>,
    >();
```

改为：

```rust
    let (wake_out_tx, wake_out_rx) = mpsc::channel::<WakeOutcome>();
```

- [ ] **步骤 2：修改 `wake_manager_loop` 的三个发送点**

`Disable` 分支：

```rust
                listener = None;
                let _ = out_tx.send(WakeOutcome::Disabled);
```

`Enable` 成功分支：

```rust
                            listener = Some(l);
                            let _ = out_tx.send(WakeOutcome::Ready(wcfg, buf, wake_rx));
```

引擎失败分支：

```rust
                        } else {
                            eprintln!("[drop-typing] 唤醒词引擎创建失败（模型缺失？）");
                            let _ = out_tx.send(WakeOutcome::Failed(
                                "唤醒词模型缺失或加载失败：请使用安装包安装；\
                                 裸 exe 需在 exe 同目录放置 models 目录，\
                                 或在设置页检查唤醒词模型目录。"
                                    .into(),
                            ));
                        }
```

监听器失败分支：

```rust
                    Err(e) => {
                        eprintln!("[drop-typing] 唤醒词监听器启动失败：{e}");
                        let _ = out_tx.send(WakeOutcome::Failed(format!(
                            "麦克风监听启动失败：{e}\
                             （请检查 Windows 设置 → 隐私 → 麦克风是否允许桌面应用访问）"
                        )));
                    }
```

- [ ] **步骤 3：修改 `run_loop` 的消费匹配**

把 `run_loop` 中 `wake_out_rx` 参数类型改为 `mpsc::Receiver<WakeOutcome>`，并把 `while let Ok(update) = wake_out_rx.try_recv()` 的匹配体改为：

```rust
            match update {
                WakeOutcome::Ready((wcfg, buf, wrx)) => {
                    wake_cfg = Some(wcfg);
                    wake_buffer = Some(buf);
                    wake_rx = Some(wrx);
                    if matches!(state, State::Idle) {
                        state = State::Listening;
                    }
                }
                WakeOutcome::Disabled => {
                    wake_cfg = None;
                    wake_buffer = None;
                    wake_rx = None;
                    if matches!(state, State::Listening) {
                        state = State::Idle;
                    }
                }
                WakeOutcome::Failed(msg) => {
                    wake_cfg = None;
                    wake_buffer = None;
                    wake_rx = None;
                    if matches!(state, State::Listening) {
                        state = State::Idle;
                    }
                    staging.error(&format!("唤醒词不可用：{msg}"));
                }
            }
```

（原 `Some(...)` / `None` 分支分别对应 `Ready` / `Disabled`。）

- [ ] **步骤 4：编译验证**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

预期：编译通过。

- [ ] **步骤 5：Commit（若采用方案 A，先单独提交用户 WIP）**

```bash
git add src-tauri/src/pipeline.rs
git commit -m "fix(wakeword): 加载/监听失败在暂存条可见提示"
```

## 任务 5：script.rs 平台化

**文件：** `src-tauri/src/script.rs`

- [ ] **步骤 1：替换家目录展开为跨平台实现**

把：

```rust
fn expand_tilde(raw: &str) -> String {
    let Some(home) = std::env::var("HOME").ok() else {
        return raw.to_string();
    };
    if raw == "~" {
        home
    } else if let Some(rest) = raw.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else {
        raw.to_string()
    }
}
```

替换为：

```rust
/// 用户主目录（macOS 用 HOME，Windows 用 USERPROFILE）。
fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

fn expand_tilde(raw: &str) -> String {
    let Some(home) = home_dir() else {
        return raw.to_string();
    };
    let home = home.to_string_lossy().into_owned();
    if raw == "~" {
        home
    } else if let Some(rest) = raw.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else {
        raw.to_string()
    }
}
```

- [ ] **步骤 2：`run_file` 支持 .bat/.cmd/.ps1**

把 `run_file` 整体替换为：

```rust
fn run_file(path: &Path) -> Result<(), ScriptError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    let mut cmd = match ext.as_deref() {
        // Windows 批处理必须经 cmd.exe 启动
        Some("bat" | "cmd") => {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(path);
            c
        }
        // PowerShell 脚本
        Some("ps1") => {
            let mut c = Command::new("powershell");
            c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(path);
            c
        }
        // 可执行文件 / shebang 脚本（macOS/Linux）
        _ => Command::new(path),
    };
    if let Some(parent) = path.parent() {
        cmd.current_dir(parent);
    }
    let output = cmd.output().map_err(|e| {
        let hint = if cfg!(windows) {
            "请确认文件关联，或改用 .bat/.cmd/.ps1/.exe".to_string()
        } else {
            "若是缺少执行权限，请先运行 chmod +x 后重试".to_string()
        };
        ScriptError::new(format!("无法执行脚本 {}：{e}（{hint}）", path.display()))
    })?;
    finish(output)
}
```

- [ ] **步骤 3：`run_shell_line` 按平台分流**

把 `run_shell_line` 整体替换为：

```rust
#[cfg(target_os = "windows")]
fn run_shell_line(line: &str) -> Result<(), ScriptError> {
    let cwd = home_dir().unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let output = Command::new("cmd")
        .args(["/C", line])
        .current_dir(cwd)
        .output()
        .map_err(|e| ScriptError::new(format!("无法启动 cmd.exe：{e}")))?;
    finish(output)
}

#[cfg(not(target_os = "windows"))]
fn run_shell_line(line: &str) -> Result<(), ScriptError> {
    let cwd = home_dir().unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let output = Command::new("/bin/zsh")
        .args(["-lc", line])
        .current_dir(cwd)
        .output()
        .map_err(|e| ScriptError::new(format!("无法启动 /bin/zsh：{e}")))?;
    finish(output)
}
```

注意：`home_dir()` 返回 `Option<PathBuf>`，两分支统一使用 `unwrap_or_else(|| ...)` 写法。

- [ ] **步骤 4：测试分平台**

把现有 `mod tests` 中 `write_exec_script` 与两个文件执行测试、`tilde_expansion_uses_home` 包一层 `#[cfg(unix)]`（`std::os::unix::fs::PermissionsExt` 仅 unix 可用）；`tilde_expansion_uses_home` 断言改为 `dirs::home_dir()`：

```rust
    #[cfg(unix)]
    #[test]
    fn tilde_expansion_uses_home() {
        let home = dirs::home_dir().expect("测试环境应有家目录");
        let home = home.to_string_lossy();
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("~/backup.sh"), format!("{home}/backup.sh"));
        assert_eq!(expand_tilde("/abs/path.sh"), "/abs/path.sh");
    }
```

并追加 Windows 测试：

```rust
    #[cfg(windows)]
    #[test]
    fn shell_line_via_cmd() {
        assert!(run("echo drop-typing-ok").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn shell_line_nonzero_exit_via_cmd() {
        let err = run("exit 7").unwrap_err();
        assert!(err.to_string().contains("退出码 7"), "{err}");
    }
```

- [ ] **步骤 5：本机（macOS）运行测试**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib script::
```

预期：unix 分支测试 PASS；Windows 专属测试不参与 macOS 编译。

- [ ] **步骤 6：Commit**

```bash
git add src-tauri/src/script.rs
git commit -m "feat(script): Windows 用 cmd.exe 执行脚本并支持 bat/ps1"
```

## 任务 6：UI 文案、设置页提示、配置示例

**文件：** `src/main.ts`、`settings.html`、`src/settings.ts`、`config.example.toml`

- [ ] **步骤 1：暂存条占位文案改为平台中立**

把 `src/main.ts:15`：

```typescript
const PLACEHOLDER = "按住右 ⌘ 说话，短按提交";
```

改为：

```typescript
const PLACEHOLDER = "按住热键说话，松开出字（短按提交）";
```

- [ ] **步骤 2：设置页长按帮助文案改为平台中立**

把 `settings.html:330`：

```html
<p class="help">按住右 ⌘ 达到该时长视为长按录音，否则短按提交。默认 150ms。</p>
```

改为：

```html
<p class="help">按住热键达到该时长视为长按录音，否则短按提交。默认 150ms。</p>
```

- [ ] **步骤 3：设置页平台提示补充「可区分左右」**

把 `src/settings.ts:1657-1658`：

```typescript
  platformHintEl.textContent = d.platform === 'macos'
    ? 'macOS：单键快捷键（如 R-CMD、R-OPT 等）'
    : 'Windows：组合快捷键（如 CMD + ALT 等）';
```

改为：

```typescript
  platformHintEl.textContent = d.platform === 'macos'
    ? 'macOS：单键快捷键（如 R-CMD、R-OPT 等）'
    : 'Windows：组合快捷键（如 CMD + ALT 等，可勾选「区分左右」精确到单侧）';
```

- [ ] **步骤 4：`config.example.toml` 更新快捷键与脚本说明**

把：

```toml
# 注意：Windows 端三个功能通道（trigger/repair/command）仅支持修饰键家族名，
# cancel 支持任意键名。
```

改为：

```toml
# 注意：Windows 端三个功能通道支持修饰键家族名（左右任意）或精确修饰键名
# （如 MetaRight/AltGr/ControlRight/ShiftRight，区分左右），cancel 支持任意键名。
```

把 script 说明：

```toml
#   - 一行 shell 命令，如 "open https://example.com"（交给 /bin/zsh -lc 执行）
```

改为：

```toml
#   - 一行 shell 命令，如 "open https://example.com"（macOS 交给 /bin/zsh -lc；
#     Windows 交给 cmd.exe /C 执行，如 "start https://example.com"）
```

- [ ] **步骤 5：前端编译验证**

```bash
npm run build
```

预期：tsc 类型检查 + vite 构建通过。

- [ ] **步骤 6：Commit**

```bash
git add src/main.ts settings.html src/settings.ts config.example.toml
git commit -m "docs(ui): Windows 文案平台中立并更新快捷键/脚本说明"
```

## 任务 7：CI 测试与文档收尾

**文件：** `.github/workflows/build-windows.yml`、`README.md`、`AGENTS.md`

- [ ] **步骤 1：workflow 增加 Rust 单元测试步骤**

在 `Build Tauri app` 之后、`Copy wakeword models` 之前插入：

```yaml
      - name: Run Rust unit tests
        run: cargo test --lib
        working-directory: src-tauri
```

（任务 5 已保证 Windows 下 `cargo test` 可编译；任务 1 的组合匹配测试、任务 3 的模型查找测试、任务 5 的 Windows 脚本测试都会在此运行。）

- [ ] **步骤 2：README Windows 章节更新**

在 README 的 Windows 说明处补充以下要点（按现有文档风格写中文）：

1. Windows 默认快捷键为 Win+Alt（录入）/ Ctrl+Alt（修复）/ Win+Shift（指令）；可在设置页勾选「区分左右」使用精确侧键。
2. App 运行期间，Win 单键、Win+E、Win+R 等系统快捷键保持可用；Win+Alt 录音时开始菜单不会弹出。
3. 默认 Win+Shift 与系统截图 Win+Shift+S 存在键位冲突，如需保留截图请修改指令通道快捷键。
4. 唤醒词模型随安装包分发；若直接使用裸 exe，需把 `models` 目录放在 exe 同目录。唤醒词加载/监听失败会在暂存条显示黄底红字。
5. script 动作：macOS 用 zsh，Windows 用 cmd.exe；`.bat/.cmd/.ps1` 文件路径可直接执行。

- [ ] **步骤 3：AGENTS.md 更新**

在 AGENTS.md 的热键/平台说明处更新：

- Windows 热键引擎支持精确左右修饰键（KeySpec::Exact），不再忽略；
- vendored rdev 的 Win 键拦截改为「仅吞属于 drop-typing 组合的 Win 键事件」，保留系统 Win 快捷键；
- 裸 exe 场景需在 exe 旁放置 `models`，否则唤醒词失败并在暂存条提示。

- [ ] **步骤 4：本地最终验证**

```bash
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

预期：全部通过（Windows 专属测试除外，由 CI 执行）。

- [ ] **步骤 5：Commit**

```bash
git add .github/workflows/build-windows.yml README.md AGENTS.md
git commit -m "ci(docs): Windows 单元测试、安装包校验与平台文档"
```

## 任务 8：Windows 手工验证清单（用户执行）

以下步骤需在真实 Windows 机器上、安装 NSIS 安装包（或下载含 `models` 的裸 exe 目录）后执行：

- [ ] 1. 启动后进入设置 → 快捷键：点「重置为平台默认」，确认 trigger 显示为 CMD+ALT、repair 为 CTRL+ALT、command 为 CMD+SHIFT。
- [ ] 2. 按一下 Win：开始菜单正常弹出；按 Win+E：资源管理器打开；按 Win+R：运行框打开。
- [ ] 3. 长按 Win+Alt：暂存条出现并显示「倾听中」，开始菜单不弹出；松开后若满足长按阈值则转写，短按则提交。
- [ ] 4. 在快捷键面板勾选「区分左右」，把 trigger 录成 R-CMD + R-OPT：按左 Win+左 Alt 不应触发，按右 Win+右 Alt 应触发。
- [ ] 5. 设置页开启唤醒词后重启：若模型缺失，暂存条应显示「唤醒词不可用」黄底红字；安装包场景下说「DT打」应进入录音。
- [ ] 6. 配置一个 script 动作（如一行 `start https://example.com`），长按指令通道说出动作名，确认能在 Windows 上执行。
- [ ] 7. 裸 exe 场景：把 `src-tauri/models` 复制为 exe 同目录的 `models`，重复第 5 步。
- [ ] 8. 把 Mac 拷贝来的 `[hotkey]` 配置在设置页确认当前生效值；如需 Windows 默认组合，直接「重置为平台默认」并重启。

## 自检记录

对照用户确认的需求逐项检查：

- 裸 exe 无法唤醒词 → 任务 3（打包/模型回退）+ 任务 4（失败可见）覆盖。
- 系统 Win 快捷键保持可用 + Win+Alt 不弹开始菜单 → 任务 2 覆盖。
- 默认不分左右、用户指定侧只认该侧 → 任务 1+2 覆盖。
- 配置里 Mac 精确键在 Windows 静默失效 → 任务 1+2 覆盖（现在会真正生效并打印警告）；设置页可重置。
- script 在 Windows 必失败 → 任务 5 覆盖。
- 占位文案写死右 ⌘ → 任务 6 覆盖。
- Windows 无法产出安装包 → 任务 3 覆盖。
- Windows `cargo test` 编译不过 → 任务 5 + 任务 7 覆盖。
