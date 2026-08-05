# drop-typing — AGENTS.md

> 本文件面向 AI 编码代理，介绍本项目的架构、构建方式与开发约定。产品细节见 [PRD.md](PRD.md)，用户文档见 [README.md](README.md)。

## 项目概述

**drop-typing** 是一款 macOS 桌面语音输入工具（MIT 协议）：**按住右 ⌘ 说话、松手出字**。ASR（语音识别）结果先进入屏幕底部的**暂存条**（staging bar），用户确认后**短按右 ⌘** 通过剪贴板 + 模拟 Cmd+V 粘贴到当前聚焦 App。

当前进度：**M4 指令通道**（M2 清洗层 + 语音修正通道 + 右 ⇧ 语音指令；设置界面/权限引导未做、暂存条只读）。后续里程碑（M3 手动编辑/自动确认、M4 设置界面、M5 打磨发布）见 PRD.md。

技术栈：

- **前端**：Vite + 原生 TypeScript（无框架），仅一个暂存条窗口（`index.html` + `src/main.ts` + `src/style.css`）
- **后端**：Rust + Tauri 2，所有业务逻辑在 Rust 侧
- **平台依赖**：macOS 专属（`macos-private-api`、rdev / enigo / arboard / cpal）

## 构建与运行命令

环境依赖：Rust stable、Node.js ≥ 20、Xcode Command Line Tools。

```bash
npm install                # 安装前端依赖
npm run tauri dev          # 开发模式（Vite dev server 固定端口 1420 + Rust）
npm run tauri build        # 打包 .app（targets = "app"）
npm run build              # 仅前端：tsc 类型检查 + vite build → dist/

# Rust 侧（在 src-tauri/ 目录下）
cargo check                # 快速编译检查
cargo run --example test_asr -- path/to/audio.wav   # ASR 独立手动测试（不需启动 App）
cargo run --example test_llm -- "要清洗的文本"      # LLM 清洗独立手动测试（两档对比）
```

首次 `cargo build` 需编译大量依赖，耗时数分钟属正常。**没有配置任何自动化测试、lint、格式化工具**；验证方式为编译 + README 的"用户验证清单"手动测试。

## 运行时配置

配置文件是**家目录点文件** `~/.drop-typing.toml`（不是项目内文件），模板见 `config.example.toml`。约定：

- `[asr].provider` 是厂商名（分组/文档用），`[asr].protocol` 决定代码用哪个协议适配器（`dashscope-realtime` 默认 / `dashscope-http` 备选）；`protocol` 缺省时按旧版 `provider` 写法向后兼容推断
- `[llm]`（M2 清洗层）同样约定：`protocol` 决定适配器（`openai-chat` 默认 / `anthropic-messages`），`strength` 为优化强度档位（`conservative` / `standard` 默认）；**不配置 `[llm]` 或缺 api_key 即关闭清洗、ASR 直出**
- API Key 可用环境变量 `DASHSCOPE_API_KEY` 提供（优先级低于配置文件，仅对 ASR 生效）
- `long_press_threshold_ms`：长按/短按判定阈值，默认 150ms
- `command_countdown_ms`：语音指令确认倒计时（M4），默认 1000ms；指令解析完成后在暂存条大字展示并倒计时，到 0 自动模拟按键

macOS 权限：辅助功能（热键监听 + 模拟粘贴）与麦克风（录音）都必须授予。**dev 模式下授权的是运行 `npm run tauri dev` 的终端**，打包后的 .app 授权 App 本身；dev 模式裸二进制没有 Info.plist，部分系统版本无法弹窗申请麦克风，遇录音为空时用打包的 .app 验证。

## 代码结构

```
index.html / src/            # 暂存条前端（Vite + 原生 TS，无框架）
│   ├── main.ts              #   事件订阅、渲染、高度自适应
│   └── style.css            #   深浅色、波形动画、黄底红字异常态
src-tauri/
├── src/
│   ├── main.rs              # 仅调用 drop_typing_lib::run()
│   ├── lib.rs               # 窗口创建（无边框/置顶/全工作区/忽略鼠标）+ 启动
│   ├── pipeline.rs          # 编排：热键 → 录音 → ASR → 清洗 → 暂存条 → 提交
│   ├── staging.rs           # 暂存条状态 + 窗口显隐/锚点定位/resize（文本归属 Rust 侧，前端只渲染）
│   ├── caret.rs             # 光标屏幕位置查询（macOS AX API，手写 extern 声明）
│   ├── config.rs            # 配置加载（[asr]/[llm] 段 + legacy/env 回退）
│   ├── command.rs           # 语音指令解析（M4）：词表驱动扫描提取（中文/谐音/填充词容忍），纯本地不过 LLM
│   ├── asr/                 # ASR 抽象：批量/实时两套 trait + 每家一个适配器
│   │   ├── bailian.rs       #   百炼 qwen3-asr-flash（HTTP 同步，备选）
│   │   └── bailian_realtime.rs  # 百炼 fun-asr-realtime（WebSocket 流式，默认）
│   ├── llm/                 # LLM 清洗抽象（M2）：trait + 每种协议一个适配器
│   │   ├── mod.rs           #   trait/Strength 档位/system prompt/pangu 兜底 + dispatch
│   │   ├── openai.rs        #   OpenAI Chat Completions 兼容（默认协议，DeepSeek 缺省端点）
│   │   └── anthropic.rs     #   Anthropic Messages 兼容（如百炼 /apps/anthropic）
│   ├── audio/recorder.rs    # cpal 录音 → 16kHz 单声道；流式 PCM chunk / 整段 WAV
│   ├── hotkey/              # trait HotkeySource（平台相关）
│   │   ├── macos.rs         #   rdev 全局监听 + 辅助功能权限检测
│   │   └── windows.rs       #   rdev 低级键盘钩子（WH_KEYBOARD_LL），修饰键组合检测
│   └── inject/              # trait Injector（平台相关）
│       ├── macos.rs         #   arboard 剪贴板 + enigo 按键模拟（Cmd+V / 任意按键组合）
│       └── windows.rs       #   剪贴板 + 模拟 Ctrl+V
├── examples/test_asr.rs     # ASR 独立手动测试入口
├── examples/test_llm.rs     # LLM 清洗独立手动测试入口
└── tauri.conf.json / capabilities/default.json / Info.plist / icons/
```

模块划分原则：

- **平台相关代码集中在 `hotkey/` 与 `inject/` 的 trait 后面**；Windows 已实现（`hotkey/windows.rs` rdev 低级键盘钩子，默认 Win+Alt 录入 / Ctrl+Alt 修复 / Win+Shift 电脑控制，避开微信语音输入的 Ctrl+Win；`inject/windows.rs` 剪贴板 + 模拟 Ctrl+V），平台依赖在 `Cargo.toml` 的 `[target.'cfg(target_os = "macos")'.dependencies]` 下按 cfg 分支添加
- **ASR 每厂商一个适配器文件**，通过 `protocol` 字段选择；新增厂商时在 `asr/` 下加文件并实现对应 trait
- **LLM 每种协议一个适配器文件**（`llm/`，M2），同样通过 `protocol` 选择；清洗失败必须降级为原文追加，不能丢内容
- 暂存条文本状态由 **Rust 侧**持有（`staging.rs`），前端只通过事件订阅渲染

## 开发约定与注意事项

- **语言**：代码注释、文档、commit 均使用中文
- **热键方案固定用 rdev**，不要换成 tauri-plugin-global-shortcut——M1 需要"裸右 ⌘ 单独按下 + 精确 press/release 事件 + 松开时长判定"，插件拿不到单独松开事件、也不支持裸修饰键语义
- **rdev 是 vendored 补丁**：`Cargo.toml` 中 `[patch.crates-io] rdev = { path = "vendor/rdev" }`，移除了 CGEventTap 后台线程中对 TIS/TSM 输入法 API 的调用（macOS 26 主线程断言导致 EXC_BREAKPOINT）。`cargo update` 时 rdev 被锁定在 0.5.3；上游修复前**不要移除该 patch**
- `tauri.conf.json` 开启了 `macOSPrivateApi`（透明/置顶窗口需要）；CSP 为 `null`；窗口名为 `staging`，权限见 `capabilities/default.json`（仅 `core:default` + `core:event:default`）
- 提交（短按）流程：暂存条 → 剪贴板 → 模拟 Cmd+V → **恢复原剪贴板** → 清空暂存条。注意剪贴板只按纯文本保存/恢复（M1 已知限制）
- **确认行为有三种，语义统一**：录入通道短按（macOS 右 ⌘ / Windows Win+Alt）、鼠标左键双击（rdev 监听 `ButtonPress(Left)`，500ms 窗口 `MOUSE_DOUBLE_CLICK_MS`，提交到鼠标所在输入框——双击已把焦点带过去；提交前先模拟一次 → 方向键折叠双击产生的选词，避免替换被选词）。**异常态（黄底红字）下任一确认行为第一次仅消除错误**（`Recording.dismiss_only` / `staging.has_error()` 判定），不提交、不清文本，无内容时顺带隐藏窗口
- **暂存条默认隐藏**（`lib.rs` 窗口 `.visible(false)`）：按下右 ⌘ 才显示——定位回退链：光标/聚焦元素（`caret.rs` AX 查询，可视条左上角对齐光标底边，注意扣除前端 #bar 的 6px CSS margin）→ 聚焦窗口内底部居中 → 屏幕底部居中；短按提交 / 录音作废时隐藏，**不做超时自动隐藏**；**`staging.error()` 会顺带显示窗口**（启动诊断依赖这条链路）且错误常显
- **`caret.rs` 查询前会给聚焦应用设置 `AXEnhancedUserInterface`**（Electron 应用如 VSCode 需要，否则拿不到文本范围）；AX 返回的矩形要做有效性检查（高度为 0 视为垃圾值）
- **AX 符号手写 extern 声明**（`#[link(name = "ApplicationServices")]`，见 `hotkey/macos.rs` / `caret.rs`），不要为此引入 accessibility crate
- 窗口定位/resize/显隐统一由 `staging.rs` 持有（锚点模式 Bottom/Caret 决定长高方向）；`lib.rs` 只负责窗口创建
- 录音期间按下任何其它键视为组合键用法，本次录音自动作废
- **指令通道（右 ⇧，M4）**：ASR 结果走 `command.rs` 本地解析（别名表 + 组合键直说，不过 LLM）→ 暂存条大字展示 + 右侧秒级倒计时 → 自动 `injector.simulate_combo`；倒计时期间按下任意右修饰键（开始新录音）通过 `pipeline` 的代次计数（`command_gen`）取消执行；按键模拟与 Cmd+V 一样必须调度到主线程（macOS 26 TSM 断言）

## 安全注意事项

- **API Key 仅存本地**：只在 `~/.drop-typing.toml` 或环境变量 `DASHSCOPE_API_KEY` 中，只发送到百炼（DashScope）接口，绝不发送到其他地方。绝不要把真实 Key 写进仓库——`.gitignore` 已忽略项目内 `config.toml`，提交前检查 `config.example.toml` 只含占位符
- 不要把 `~/.drop-typing.toml`（用户真实配置）读入对话或复制进仓库
- 前端仅一个本地窗口、无远程内容，但改动 CSP / capabilities 时保持最小权限原则
