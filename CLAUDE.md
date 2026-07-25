# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概述

**drop-typing** 是一款 macOS 桌面语音输入工具（MIT 协议）：**按住右 ⌘ 说话、松手出字**。ASR 结果先进入屏幕底部的暂存条（staging bar），用户确认后短按右 ⌘ 通过剪贴板 + 模拟 Cmd+V 粘贴到当前聚焦 App。

当前进度：**M2**（LLM 清洗层 + 优化强度档位；无设置界面、暂存条只读）。

技术栈：
- **前端**：Vite + 原生 TypeScript（无框架），仅一个暂存条窗口
- **后端**：Rust + Tauri 2，所有业务逻辑在 Rust 侧
- **平台**：macOS 专属（`macos-private-api`、rdev / enigo / arboard / cpal）

## 构建与运行

环境依赖：Rust stable、Node.js ≥ 20、Xcode Command Line Tools。

```bash
npm install                # 安装前端依赖
npm run tauri dev          # 开发模式（Vite dev server 端口 1420 + Rust）
npm run tauri build        # 打包 .app
npm run build              # 仅前端：tsc 类型检查 + vite build → dist/

# Rust 侧（在 src-tauri/ 目录下）
cargo check                # 快速编译检查
cargo run --example test_asr -- path/to/audio.wav   # ASR 独立测试
cargo run --example test_llm -- "要清洗的文本"      # LLM 清洗独立测试（两档对比）
```

首次 `cargo build` 需编译大量依赖，耗时数分钟。**没有自动化测试、lint、格式化工具**；验证方式为编译 + README 的"用户验证清单"手动测试。

## 运行时配置

配置文件路径：**`~/.drop-typing.toml`**（家目录点文件），模板见 `config.example.toml`。

关键约定：
- `[asr].provider` 是厂商名（分组/文档用），`[asr].protocol` 决定代码用哪个协议适配器（`dashscope-realtime` 默认 / `dashscope-http` 备选）；`protocol` 缺省时按旧版 `provider` 写法向后兼容推断
- `[llm]`（M2 清洗层）同样约定：`protocol` 决定适配器（`openai-chat` 默认 / `anthropic-messages`），`strength` 为优化强度档位（`conservative` / `standard` 默认）；**不配置 `[llm]` 或缺 api_key 即关闭清洗、ASR 直出**
- API Key 可用环境变量 `DASHSCOPE_API_KEY` 提供（仅对 ASR 生效，优先级低于配置文件）
- `long_press_threshold_ms`：长按/短按判定阈值，默认 250ms

macOS 权限：辅助功能（热键监听 + 模拟粘贴）与麦克风（录音）都必须授予。**dev 模式下授权的是运行 `npm run tauri dev` 的终端**；dev 模式裸二进制没有 Info.plist，部分系统版本无法弹窗申请麦克风，遇录音为空时用打包的 .app 验证。

## 代码架构

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
│   ├── asr/                 # ASR 抽象：批量/实时两套 trait + 每家一个适配器
│   │   ├── bailian.rs       #   百炼 qwen3-asr-flash（HTTP 同步，备选）
│   │   └── bailian_realtime.rs  # 百炼 fun-asr-realtime（WebSocket 流式，默认）
│   ├── llm/                 # LLM 清洗抽象（M2）：trait + 每种协议一个适配器
│   │   ├── mod.rs           #   trait/Strength 档位/system prompt/pangu 兜底 + dispatch
│   │   ├── openai.rs        #   OpenAI Chat Completions 兼容（默认协议，DeepSeek 缺省端点）
│   │   └── anthropic.rs     #   Anthropic Messages 兼容（如百炼 /apps/anthropic）
│   ├── audio/recorder.rs    # cpal 录音 → 16kHz 单声道；流式 PCM chunk / 整段 WAV
│   ├── hotkey/              # trait HotkeySource（平台相关）
│   │   └── macos.rs         #   rdev 全局监听 + 辅助功能权限检测
│   └── inject/              # trait Injector（平台相关）
│       └── macos.rs         #   arboard 剪贴板 + enigo 模拟 Cmd+V
├── examples/test_asr.rs     # ASR 独立手动测试入口
├── examples/test_llm.rs     # LLM 清洗独立手动测试入口
└── tauri.conf.json / capabilities/default.json / Info.plist / icons/
```

### 核心设计原则

- **平台相关代码集中在 `hotkey/` 与 `inject/` 的 trait 后面**；Windows 移植只需各加一个实现文件，平台依赖在 `Cargo.toml` 的 `[target.'cfg(target_os = "macos")'.dependencies]` 下按 cfg 分支添加
- **ASR 每厂商一个适配器文件**，通过 `protocol` 字段选择
- **LLM 每种协议一个适配器文件**，同样通过 `protocol` 选择；清洗失败必须降级为原文追加，不能丢内容
- **暂存条文本状态由 Rust 侧持有**（`staging.rs`），前端只通过事件订阅渲染
- **`pipeline.rs` 是编排核心**：热键事件循环 → 录音状态机（Idle/Recording）→ 长短按判定 → ASR → 清洗 → 暂存条

### 长短按时序（pipeline.rs 状态机）

同一个右 ⌘ 上叠两个动作，**判定放在松手时**：
- 按下时长 < 250ms → 短按：提交暂存条（暂存条 → 剪贴板 → Cmd+V → 恢复原剪贴板 → 清空）
- 按下时长 ≥ 250ms → 长按：录音 → 松手后送 ASR → 清洗 → 追加到暂存条
- 录音期间按下任何其它键视为组合键用法（如 ⌘Space），本次录音作废

### 暂存条定位回退链（staging.rs）

按下右 ⌘ 开始录音时显示暂存条，定位优先级：
1. 光标/插入点位置（`caret.rs` AX 查询，可视条左上角对齐光标底边，扣除 6px CSS margin）
2. 聚焦元素下方
3. 聚焦窗口内底部居中
4. 屏幕底部居中

暂存条默认隐藏，短按提交/录音作废时隐藏；**不做超时自动隐藏**。`staging.error()` 会顺带显示窗口。

## 开发约定

- **代码注释、文档、commit 均使用中文**
- **热键方案固定用 rdev**，不要换成 tauri-plugin-global-shortcut——M1 需要裸右 ⌘ 单独按下 + 精确 press/release 事件 + 松开时长判定，插件拿不到单独松开事件
- **rdev 是 vendored 补丁**：`Cargo.toml` 中 `[patch.crates-io] rdev = { path = "vendor/rdev" }`，移除了 CGEventTap 后台线程中对 TIS/TSM 输入法 API 的调用（macOS 26 主线程断言导致 EXC_BREAKPOINT）。上游修复前不要移除该 patch
- **提交流程**：暂存条 → 剪贴板 → 模拟 Cmd+V → **恢复原剪贴板** → 清空暂存条。剪贴板只按纯文本保存/恢复（M1 已知限制）
- **`caret.rs` 查询前会给聚焦应用设置 `AXEnhancedUserInterface`**（Electron 应用如 VSCode 需要）；AX 返回的矩形要做有效性检查（高度为 0 视为垃圾值）
- **AX 符号手写 extern 声明**（`#[link(name = "ApplicationServices")]`），不要为此引入 accessibility crate
- 窗口定位/resize/显隐统一由 `staging.rs` 持有；`lib.rs` 只负责窗口创建
- `tauri.conf.json` 开启了 `macOSPrivateApi`（透明/置顶窗口需要）；CSP 为 `null`；窗口名为 `staging`

## 安全注意事项

- **API Key 仅存本地**：只在 `~/.drop-typing.toml` 或环境变量 `DASHSCOPE_API_KEY` 中，只发送到百炼接口。不要把真实 Key 写进仓库——`.gitignore` 已忽略项目内 `config.toml`，提交前检查 `config.example.toml` 只含占位符
- 不要把 `~/.drop-typing.toml`（用户真实配置）读入对话或复制进仓库
- 前端仅一个本地窗口、无远程内容，改动 CSP / capabilities 时保持最小权限原则
