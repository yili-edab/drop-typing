# 跨平台快捷键显示与唤醒词调整设计

> 状态：已获用户口头批准（2026-08-06），批准后用户指示直接开始实施。

## 背景

drop-typing 同时支持 macOS 与 Windows。当前设置页的快捷键显示固定使用 macOS 命名（CMD/OPT），Windows 用户看到的是不认识的键名；默认唤醒词仍是旧品牌词（DT打/DT修/DT控/DT确认/DT清空）；指令通道进入时不会清空暂存条旧内容；设置页第二个菜单名「快捷键」需改名。

## 目标

1. 「键盘选择」等所有前端组合键展示按运行平台显示：Windows 用 ALT / WIN，macOS 保持 CMD / OPT。
2. 默认唤醒词改为：小易记（录入）、小易修（修复）、小易控（指令）、小易确认（提交）、小易清空（清空），Mac + Windows 一致。
3. 进入控制通道（指令录音）时清空暂存条正文。
4. 设置页第二个菜单「快捷键」改为「功能热键」。

## 变更设计

### 1. 组合键显示平台化（仅显示层）

配置文件与后端仍使用规范名（`Command/Option/MetaLeft/AltGr` 等），后端在 Windows 上物理映射 Meta→Win 键、Option→Alt 键，语义本就跨平台。前端只改显示：

- 新增 `WINDOWS_MOD_DISPLAY` 映射与 `modDisplay()` / `isWindowsUI()` 两个 helper，所有展示路径统一走 `modDisplay()`。
- Windows 映射：Command/Cmd/Meta → WIN，MetaLeft/MetaRight → L-WIN/R-WIN，Option/Opt → ALT，Alt → L-ALT，AltGr → R-ALT；Ctrl/Shift 不变。
- 修饰键别名下拉（语音控制面板）的选项文字 Windows 下显示 Win/Alt，内部 value 仍是 `Cmd/Opt`，保存与后端解析不受影响。
- Windows 平台提示文案改为「WIN + ALT」。
- 语音控制面板两条静态示例按平台切换（macOS：Shift+Cmd+4 / 识别为 Cmd；Windows：Shift+Win+4 / 识别为 Win）。

平台判断：优先用后端返回的 `shortcutState.platform`，未加载时用 `navigator.userAgent` 兜底。

### 2. 默认唤醒词改为小易系

- `wakeword/mod.rs` 的 `BUILTIN_DEFAULTS` 改为 5 个新词。
- `settings.rs` 设置页默认列表同步。
- `phoneme.rs` 硬编码参考条目与测试同步（更新为 5 条）。
- 模型目录 `keywords.txt`（当前动态生成路径下已不使用，但保持一致性）同步更新。
- 前端 placeholder 与重置确认文案同步。
- `config.example.toml`、相关 docs 注释同步。
- 新增单元测试验证：默认词表内容正确、五个新词可由内置模型 text2token 转换出预期音素。

已保存的自定义唤醒词不受影响；「重置」后恢复新默认。

### 3. 控制通道清空暂存条

两条入口在进入 `RecordMode::Command` 录音时调用 `staging.take()` 清空正文：

- 热键路径（macOS 右 ⇧ / Windows 指令组合键）按下时。
- 唤醒词「小易控」触发 `start_wake_recording` 时。

录入/修复通道行为不变。该行为位于 pipeline 事件循环内，Tauri `Staging` 依赖 AppHandle 不易单测，验证方式为编译 + 人工清单。

### 4. 设置菜单改名

`settings.html` 导航 `<li data-panel="shortcut">快捷键</li>` → `功能热键`。面板 id、事件名、变量名均不动。

## 涉及文件

- `src-tauri/src/wakeword/mod.rs`、`src-tauri/src/wakeword/phoneme.rs`、`src-tauri/src/wakeword/text2token.rs`
- `src-tauri/src/settings.rs`、`src-tauri/src/pipeline.rs`
- `src-tauri/models/builtin/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20/keywords.txt`
- `src/settings.ts`、`settings.html`
- `config.example.toml`、`docs/settings-page-design.md`、`docs/wake-word-mac.md`

## 验证

- `cargo test --lib`（唤醒词默认值/转换测试）
- `cargo check`（Rust 编译）
- `npm run build`（tsc + vite 构建）
- 人工核对：Windows 下打开「功能热键 → 键盘选择」显示 ALT/WIN；macOS 显示 CMD/OPT；重置唤醒词显示小易系；进入指令通道旧文本被清空。
