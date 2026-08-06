# 跨平台快捷键显示与唤醒词调整 实现计划

> **面向 AI 代理的工作者：** 本计划在当前会话内联执行（用户已指示「开始实施」，且工作区已有未提交改动，不使用独立 worktree）。步骤使用复选框（`- [ ]`）语法跟踪进度。

**目标：** 设置页组合键显示按平台切换（Windows 用 ALT/WIN）、默认唤醒词改为小易系、控制通道清空暂存条、设置菜单改名「功能热键」。

**架构：** 显示层平台化（前端 helper 统一映射，存储/后端规范名不动）；默认值集中在 Rust 常量与设置接口；控制通道清空在 pipeline 两条入口各加一次 `staging.take()`。

**技术栈：** Rust + Tauri 2（后端/默认值）、Vite + 原生 TypeScript（设置页显示层）、sherpa-onnx text2token（唤醒词音素）。

---

### 任务 1：Rust 默认唤醒词改为小易系（TDD）

**文件：**
- 修改：`src-tauri/src/wakeword/mod.rs:80-84`（BUILTIN_DEFAULTS）
- 修改：`src-tauri/src/settings.rs:388-392`（设置页默认列表）
- 修改：`src-tauri/src/wakeword/phoneme.rs`（参考条目 + 测试）
- 修改：`src-tauri/src/wakeword/text2token.rs`（注释示例 + 新增转换测试）
- 修改：`src-tauri/models/builtin/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20/keywords.txt`

- [ ] **步骤 1：编写失败的默认值测试**

在 `src-tauri/src/wakeword/mod.rs` 末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_defaults_are_xiaoyi_words() {
        let texts: Vec<&str> = BUILTIN_DEFAULTS.iter().map(|(t, _)| *t).collect();
        assert_eq!(
            texts,
            vec!["小易记", "小易修", "小易控", "小易确认", "小易清空"]
        );
        let actions: Vec<&str> = BUILTIN_DEFAULTS.iter().map(|(_, a)| *a).collect();
        assert_eq!(actions, vec!["input", "repair", "command", "commit", "clear"]);
    }
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --lib wakeword::tests::builtin_defaults_are_xiaoyi_words`
预期：FAIL（当前默认值是 DT打 等）

- [ ] **步骤 3：修改默认值**

`mod.rs`：

```rust
const BUILTIN_DEFAULTS: &[(&str, &str)] = &[
    ("小易记", "input"),
    ("小易修", "repair"),
    ("小易控", "command"),
    ("小易确认", "commit"),
    ("小易清空", "clear"),
];
```

同步更新 `mod.rs` 顶部与 `resolve_keywords` 注释中的 DT 字样。

`settings.rs` 默认列表改为：

```rust
serde_json::json!({ "keyword": "小易记", "action": "input" }),
serde_json::json!({ "keyword": "小易修", "action": "repair" }),
serde_json::json!({ "keyword": "小易控", "action": "command" }),
serde_json::json!({ "keyword": "小易确认", "action": "commit" }),
serde_json::json!({ "keyword": "小易清空", "action": "clear" }),
```

- [ ] **步骤 4：更新 phoneme.rs 参考条目与测试**

```rust
const DEFAULT_ENTRIES: &[(&str, &str, &str)] = &[
    ("小易记", "x iǎo y ì j ì @小易记", "input"),
    ("小易修", "x iǎo y ì x iū @小易修", "repair"),
    ("小易控", "x iǎo y ì k òng @小易控", "command"),
    ("小易确认", "x iǎo y ì q uè r èn @小易确认", "commit"),
    ("小易清空", "x iǎo y ì q īng k ōng @小易清空", "clear"),
];
```

测试改为断言 5 条且首条为「小易记」。

- [ ] **步骤 5：新增五个新词的 text2token 转换测试**

在 `text2token.rs` 测试模块新增：

```rust
#[test]
fn test_convert_xiaoyi_builtin_keywords() {
    let model_dir = builtin_model_dir();
    let t2t = Text2Token::load(&model_dir).expect("加载 text2token");
    let expected = [
        ("小易记", "x iǎo y ì j ì @小易记"),
        ("小易修", "x iǎo y ì x iū @小易修"),
        ("小易控", "x iǎo y ì k òng @小易控"),
        ("小易确认", "x iǎo y ì q uè r èn @小易确认"),
        ("小易清空", "x iǎo y ì q īng k ōng @小易清空"),
    ];
    for (keyword, expected_line) in expected {
        let line = t2t.convert(keyword, keyword).expect("转换");
        assert_eq!(line, expected_line);
    }
}
```

同时把 `test_write_keywords_txt` 的数据与断言改为上述 5 条。

- [ ] **步骤 6：更新模型目录 keywords.txt**

```
x iǎo y ì j ì @小易记
x iǎo y ì x iū @小易修
x iǎo y ì k òng @小易控
x iǎo y ì q uè r èn @小易确认
x iǎo y ì q īng k ōng @小易清空
```

- [ ] **步骤 7：运行测试**

运行：`cargo test --lib wakeword`
预期：全部 PASS（含默认值、转换、原有 DT 兼容测试）

- [ ] **步骤 8：Commit**

```bash
git add src-tauri/src/wakeword/mod.rs src-tauri/src/wakeword/phoneme.rs src-tauri/src/wakeword/text2token.rs src-tauri/src/settings.rs src-tauri/models/builtin/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20/keywords.txt
git commit -m "feat(wakeword): 默认唤醒词改为小易记/小易修/小易控/小易确认/小易清空"
```

### 任务 2：控制通道进入时清空暂存条

**文件：**
- 修改：`src-tauri/src/pipeline.rs`（热键入口 ~651 行、`start_wake_recording` ~1275 行）

- [ ] **步骤 1：热键入口清空**

在热键按下路径的 `staging.clear_command();` 之后追加：

```rust
// 控制通道：进入即清空之前的暂存条内容（避免旧文本混在指令展示后）
if mode == RecordMode::Command {
    staging.take();
}
```

- [ ] **步骤 2：唤醒词入口清空**

在 `start_wake_recording` 的 `staging.clear_error();` 之后追加：

```rust
// 控制通道：进入即清空之前的暂存条内容
if mode == RecordMode::Command {
    staging.take();
}
```

- [ ] **步骤 3：编译验证**

运行：`cargo check`
预期：exit 0

- [ ] **步骤 4：Commit**

```bash
git add src-tauri/src/pipeline.rs
git commit -m "feat(pipeline): 进入控制通道时清空暂存条旧内容"
```

### 任务 3：设置页前端平台化显示 + 菜单改名 + 唤醒词文案

**文件：**
- 修改：`src/settings.ts`（`MOD_DISPLAY` 附近新增映射与 helper；`formatComboText`/`formatShortcut`/`renderKeyboard`/修饰键下拉/平台提示/唤醒词文案）
- 修改：`settings.html`（菜单名、两条语音控制示例加 id）

- [ ] **步骤 1：新增平台映射与 helper**

在 `MOD_DISPLAY` 后新增：

```ts
const WINDOWS_MOD_DISPLAY: Record<string, string> = {
  Control: 'CTRL', ControlLeft: 'L-CTRL', ControlRight: 'R-CTRL', Ctrl: 'CTRL',
  Option: 'ALT', Alt: 'L-ALT', AltGr: 'R-ALT', Opt: 'ALT',
  Shift: 'SHIFT', ShiftLeft: 'L-SHIFT', ShiftRight: 'R-SHIFT',
  Command: 'WIN', Meta: 'WIN', MetaLeft: 'L-WIN', MetaRight: 'R-WIN', Cmd: 'WIN',
};

function isWindowsUI(): boolean {
  return shortcutState.platform === 'windows' || /Windows/i.test(navigator.userAgent);
}

function modDisplay(name: string): string {
  const map = isWindowsUI() ? WINDOWS_MOD_DISPLAY : MOD_DISPLAY;
  return map[name] || name.toUpperCase();
}

function modOptionLabel(value: string): string {
  if (!isWindowsUI()) return value;
  if (value === 'Cmd') return 'Win';
  if (value === 'Opt') return 'Alt';
  return value;
}
```

- [ ] **步骤 2：统一显示路径**

- `formatComboText`：`m => MOD_DISPLAY[m] || m.toUpperCase()` → `m => modDisplay(m)`
- `formatShortcut`：`if (MOD_DISPLAY[k]) return MOD_DISPLAY[k];` → `if (modDisplay(k) !== k.toUpperCase()) return modDisplay(k);`（保留后续 KEY_DISPLAY_UNIFIED/Key/Num 处理）
- `renderKeyboard`：`btn.textContent = MOD_DISPLAY[modValue] || modValue;` → `btn.textContent = modDisplay(modValue);`
- 修饰键别名下拉：`o.textContent = m;` → `o.textContent = modOptionLabel(m);`

- [ ] **步骤 3：平台提示与语音控制示例**

`shortcut-config` 监听中 Windows 文案改为：

```ts
: 'Windows：组合快捷键（如 WIN + ALT 等，可勾选「区分左右」精确到单侧）';
```

`settings.html` 两条示例加 id 并留默认 macOS 文案：

```html
<p class="help" id="lex-action-help">说「截图」→ 按 Shift+Cmd+4</p>
<p class="help" id="lex-modifier-help">说「win」→ 识别为 Cmd</p>
```

`settings.ts` 新增并在模块加载与 `shortcut-config` 回调中调用：

```ts
function applyPlatformHelp() {
  const win = isWindowsUI();
  const actionHelp = document.getElementById('lex-action-help');
  const modHelp = document.getElementById('lex-modifier-help');
  if (actionHelp) actionHelp.textContent = win
    ? '说「截图」→ 按 Shift+Win+4'
    : '说「截图」→ 按 Shift+Cmd+4';
  if (modHelp) modHelp.textContent = win
    ? '说「win」→ 识别为 Win'
    : '说「win」→ 识别为 Cmd';
}
```

- [ ] **步骤 4：菜单改名**

`settings.html`：`<li data-panel="shortcut">快捷键</li>` → `<li data-panel="shortcut">功能热键</li>`

- [ ] **步骤 5：唤醒词前端文案**

- placeholder：`'例如 DT打'` → `'例如 小易记'`
- 重置确认：`'确认将唤醒词重置为默认值（DT打/DT修/DT控）？当前修改将丢失。'` → `'确认将唤醒词重置为默认值（小易记/小易修/小易控/小易确认/小易清空）？当前修改将丢失。'`

- [ ] **步骤 6：构建验证**

运行：`npm run build`
预期：tsc 无错误，vite build exit 0

- [ ] **步骤 7：Commit**

```bash
git add src/settings.ts settings.html
git commit -m "feat(settings): 快捷键显示按平台切换（Windows 用 WIN/ALT）并更名功能热键"
```

### 任务 4：文档与配置示例同步

**文件：**
- 修改：`config.example.toml:82-100`
- 修改：`docs/settings-page-design.md:135,239`
- 修改：`docs/wake-word-mac.md:130`

- [ ] **步骤 1：替换所有 DT 唤醒词字样**

将上述文件中的 `DT打/DT修/DT控/DT确认/DT清空` 统一替换为 `小易记/小易修/小易控/小易确认/小易清空`（示例与代码默认值一致）。

- [ ] **步骤 2：核对**

运行：`rg -n "DT打|DT修|DT控|DT确认|DT清空" --glob '!docs/superpowers/**' --glob '!*.onnx' --glob '!*.wav' --glob '!src-tauri/target/**' --glob '!node_modules/**'`
预期：仅历史计划/模型内未使用文件可能残留，代码与面向用户的文档无残留。

- [ ] **步骤 3：Commit**

```bash
git add config.example.toml docs/settings-page-design.md docs/wake-word-mac.md
git commit -m "docs: 默认唤醒词改为小易系"
```

### 任务 5：全量验证

- [ ] **步骤 1：Rust 测试**

运行：`cargo test --lib`
预期：全部 PASS

- [ ] **步骤 2：Rust 编译**

运行：`cargo check`
预期：exit 0

- [ ] **步骤 3：前端构建**

运行：`npm run build`
预期：exit 0

- [ ] **步骤 4：需求核对**

逐项核对设计文档四个目标是否都有对应实现与验证。
