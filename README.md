# drop-typing

<img src="docs/logo-from-mascot-1.png" alt="易达熊打字 Logo" width="300" height="300" />

> **说得轻松，写得完美，活得快乐**
>
> **Speak freely, write perfectly, live happily**

**易达熊打字**（drop-typing）是易达熊公司旗下的开源子产品线，一款多端的语音输入工具，当前支持 Macos/windows。我们坚持以开源为核心，后续也会提供商业化版本——主要面向希望开箱即用的用户，提供更加便捷的云端语音识别与大模型接入能力。商业化版本与开源版本在核心体验上没有本质区别。

## 核心功能

1. **语音识别打字**
   - **基础润色**：自动去除语气词、修正标点、优化中英混排空格、结构化输出，每个方向都可根据个人偏好调整。
   - **高级润色**：可自定义润色规则与强度，接入 OpenAI / DeepSeek / Qwen / Anthropic 等兼容大模型进行深度文本优化。

2. **语音修复**
   - 对着已输入的内容说出修复意见，即可在原地进行语音驱动的文本修正，改错、改写、补全都能说。

3. **语音控制**
   - 自定义丰富的快捷键词表，通过语音直接唤醒系统或应用操作，例如「微信截图」「复制」「粘贴」「全选」等，边说边控。

---

> 按住说话、松手出字的语音输入工具（macOS）。当前进度：**M4 指令通道**（设置界面/权限引导未做）。
>
> 开源协议 MIT。产品细节见 [PRD.md](PRD.md)。

## M4 已实现（语音指令通道，右 ⇧）

- 长按**右 ⇧** 录指令语音 → ASR 转写 → **本地解析**为按键组合（不经过 LLM，PRD 4.3）
- 识别期间暂存条状态徽章显示「指令识别中」，中间结果实时展示（与输入通道一致）
- 解析完成后：暂存条**大字等宽字体**展示指令（如 `CMD+C`），右侧**秒级倒计时**，到 0 自动模拟按键；倒计时期间按下任意右修饰键（开始新录音）即取消执行
- 倒计时时长可配置：`command_countdown_ms`（默认 1000）
- 内置别名（macOS 上映射为 ⌘）：复制 / 粘贴 / 剪切 / 撤销 / 重做 / 全选 / 保存 / 回车（含 copy / paste / ctrl+c / ctrl+v / enter 等英文说法）
- 组合键直说：如 "Shift Command E" → `SHIFT+CMD+E`；词表驱动扫描提取（非整句精确匹配），容忍中文说法（命令/控制/换挡/选项）、填充词（"按一下"）、连接词（"加/和/与"）、字母谐音（"西"→C）等 ASR 变形；unknown 占比过半判废防误触发
- 动作别名支持脚本执行：`script` 可填已存在的脚本文件路径（绝对路径或 `~` 开头，按 shebang 执行、需可执行权限），也可直接填一行 shell 命令（如 `open https://example.com`）；一行命令的解释器可用 `shell` 字段选择（Windows `cmd` / `powershell`，macOS 固定 `zsh`）；与按键指令同样有倒计时确认，失败时黄底红字显示退出码与错误信息
- 未命中解析：黄底红字提示「未识别到按键指令」；右 ⇧ 短按无动作
- 按键模拟复用 macOS 主线程调度约束（macOS 26 TSM 断言），支持字母/数字/F 键/方向键/Space/Tab/Esc/Enter/Delete

## M2 已实现

- LLM 清洗层（`llm/` 模块，与 `asr/` 同构的 trait + 协议适配器）：
  - `openai-chat`（默认协议）：OpenAI Chat Completions 兼容，一套实现兼容 DeepSeek / OpenAI / Qwen / Ollama
  - `anthropic-messages`：Anthropic Messages API 兼容（如百炼 `/apps/anthropic` 端点）
- 清洗规则（PRD 4.1）：标点修正、去口水话、中英混排空格（LLM 为主 + 本地 pangu 兜底后处理）、口语结构化
- 优化强度档位：`[llm].strength = "conservative"`（只加标点）/ `"standard"`（默认，全规则）
- 清洗失败降级为原文追加 + 黄底红字提示，不丢内容；未配置 `[llm]` 或缺 api_key 时清洗层关闭、ASR 直出
- 暂存条按需显示：默认隐藏，按下右 ⌘ 开始录音时才出现；**有光标则可视条左上角紧贴光标底边**（macOS Accessibility API 取插入点位置），取不到光标时依次回退到聚焦元素下方 → 聚焦窗口内底部居中 → 屏幕底部居中；短按提交后立即隐藏；错误（黄底红字）常显不自动隐藏
- 右侧状态徽章：倾听中 → 识别中 → 润色中；润色期间先以未定稿样式显示未润色原文，完成后替换为润色结果

## M1 已实现

- Tauri 2 脚手架（Vite + 原生 TypeScript 前端，无框架）
- 暂存条浮层：屏幕底部居中、无边框、置顶、全工作区可见、不抢焦点（忽略鼠标事件）、多行自适应高度、深浅色跟随系统
- 录音反馈：CSS 波形动画（无提示音）；异常时整条黄底红字
- 右 ⌘ 全局热键（rdev）：按下即录音，松开判定 —— ≥250ms 长按送 ASR、<250ms 短按提交；录音期间按其它键视为组合键用法自动作废
- 录音：cpal → 16kHz 单声道（边录边流式输出 PCM chunk；批量路径输出 WAV）
- ASR（默认）：阿里百炼 **fun-asr-realtime**（DashScope 原生 WebSocket 流式协议），边录边传边出字——中间结果实时显示在暂存条，松开后取最终全文**追加**到暂存条（M1 不做 LLM 清洗）
- ASR（备选）：阿里百炼 qwen3-asr-flash（DashScope HTTP 同步接口，`protocol = "dashscope-http"`）
- 短按提交：暂存条 → 剪贴板 → 模拟 Cmd+V → 恢复原剪贴板 → 清空暂存条
- 配置：家目录点文件 `~/.drop-typing.toml`，缺失/无权限时暂存条黄底红字提示

## 环境依赖

| 依赖 | 安装 |
|------|------|
| Rust（stable） | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh -s -- -y` |
| Node.js ≥ 20 | 任意方式（本项目用 v24 验证） |
| Xcode Command Line Tools | `xcode-select --install` |

## 安装与运行

```bash
git clone <repo> && cd drop-typing
npm install
npm run tauri dev
```

首次 `cargo build` 会编译大量依赖，耗时数分钟属正常。

## 配置 API Key

配置文件路径：家目录点文件 `~/.drop-typing.toml`。

```bash
cp config.example.toml ~/.drop-typing.toml
# 编辑文件，填入 [asr].api_key（https://bailian.console.aliyun.com/ 获取）
```

最小配置：

```toml
[asr]
provider = "bailian"                  # 厂商名
protocol = "dashscope-realtime"       # 协议适配器：WebSocket 流式（默认）；备选 "dashscope-http"（HTTP 整段）
model = "fun-asr-realtime"
base_url = "wss://dashscope.aliyuncs.com/api-ws/v1/inference"   # 工作区 compatible-mode 地址也可，会自动推导
api_key = "sk-xxx"
```

约定：`provider` 是厂商名（分组/文档用），`protocol` 决定代码使用哪个协议适配器；`protocol` 缺省时按旧版 `provider` 写法（如 `bailian-realtime`）向后兼容推断。

也可以用环境变量代替配置文件中的 Key：`export DASHSCOPE_API_KEY=sk-xxx`。

API Key 仅存本地，绝不上传到除百炼接口以外的任何地方。

### LLM 清洗（可选）

配置 `[llm]` 段即开启清洗；不配置或缺 `api_key` 则 ASR 直出（关闭清洗）：

```toml
[llm]
provider = "deepseek"
protocol = "openai-chat"          # 缺省即 openai-chat；备选 "anthropic-messages"
api_key = "sk-xxx"
# strength = "standard"           # conservative（只加标点）/ standard（默认，全规则）
```

- `openai-chat`：`base_url` / `model` 缺省为 DeepSeek 官方端点（`https://api.deepseek.com` / `deepseek-chat`），换成 OpenAI / Qwen / Ollama 时显式填写即可
- `anthropic-messages`：需显式配置 `base_url` 与 `model`（百炼 Anthropic 兼容端点示例见 `config.example.toml`）

手动测试清洗（不启动 App）：

```bash
cargo run --example test_llm -- "嗯 那个 我觉得吧 这个功能 第一 要快 第二 要稳"
```

## 权限说明（macOS）

| 权限 | 用途 | 说明 |
|------|------|------|
| 辅助功能（Accessibility） | 全局热键监听（rdev / CGEventTap）+ 模拟 Cmd+V（enigo / CGEvent） | 系统设置 → 隐私与安全性 → 辅助功能。**dev 模式下要授权的是运行 `npm run tauri dev` 的终端**（如 Terminal / iTerm / Kimi）；打包后的 .app 则授权 App 本身。授权后需重启应用。 |
| 麦克风 | cpal 录音 | 打包的 .app 会通过 Info.plist 中的 `NSMicrophoneUsageDescription` 弹窗申请。**dev 模式下裸二进制没有 Info.plist，部分系统版本无法弹窗授权**——如遇录音为空，请 `npm run tauri build` 出 .app 后运行验证，或在 系统设置 → 麦克风 中确认终端已被列出。 |

未授予辅助功能权限时，暂存条会显示黄底红字提示；热键与粘贴不会工作，但应用不会崩溃。

## 手动测试 ASR（不需要启动 App）

```bash
cargo run --example test_asr -- path/to/audio.wav
```

- 默认走 realtime 路径：WAV 须为 16kHz 单声道 16bit（可用 `say -o test.wav --data-format=LEI16@16000 "你好，世界"` 生成），会打印中间结果与最终全文
- `protocol = "dashscope-http"` 时整段原字节上传。Key 也可写在配置文件里

## 代码结构

```
drop-typing/
├── index.html / src/            # 暂存条前端（Vite + 原生 TS）
│   ├── main.ts                  #   事件订阅、渲染、高度自适应
│   └── style.css                #   深浅色、波形动画、黄底红字异常态
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs               # 窗口创建（无边框/置顶/全工作区/忽略鼠标）+ 启动
│   │   ├── pipeline.rs          # 编排：热键 → 录音 → ASR → 清洗 → 暂存条 → 提交
│   │   ├── staging.rs           # 暂存条状态 + 窗口显隐/锚点定位（文本归属 Rust 侧）
│   │   ├── caret.rs             # 光标屏幕位置查询（macOS AX API，贴光标显示）
│   │   ├── config.rs            # 配置加载（[asr]/[llm] 段 + legacy/env 回退）
│   │   ├── asr/                 # ASR 抽象：批量/实时两套 trait + 每家一个适配器
│   │   │   ├── mod.rs
│   │   │   ├── bailian.rs       #   百炼 qwen3-asr-flash（HTTP，备选）
│   │   │   └── bailian_realtime.rs # 百炼 fun-asr-realtime（WebSocket 流式，默认）
│   │   ├── llm/                 # LLM 清洗抽象：trait + 每种协议一个适配器
│   │   │   ├── mod.rs           #   trait/档位/prompt/pangu 兜底 + dispatch
│   │   │   ├── openai.rs        #   OpenAI 兼容协议（默认，DeepSeek 缺省端点）
│   │   │   └── anthropic.rs     #   Anthropic Messages 兼容协议
│   │   ├── audio/recorder.rs    # cpal 录音 → 流式 PCM chunk / 整段 WAV
│   │   ├── hotkey/              # 热键抽象：trait HotkeySource（平台相关）
│   │   │   ├── mod.rs
│   │   │   ├── macos.rs         #   rdev 全局监听 + 辅助功能权限检测
│   │   │   └── windows.rs       #   rdev 低级键盘钩子（WH_KEYBOARD_LL），修饰键组合检测
│   │   └── inject/              # 注入抽象：trait Injector（平台相关）
│   │       ├── mod.rs
│   │       ├── macos.rs         #   arboard 剪贴板 + enigo 模拟 Cmd+V
│   │       └── windows.rs       #   剪贴板 + 模拟 Ctrl+V
│   ├── examples/test_asr.rs     # ASR 独立手动测试入口
│   ├── examples/test_llm.rs     # LLM 清洗独立手动测试入口
│   └── tauri.conf.json / capabilities/ / Info.plist
├── config.example.toml
└── PRD.md
```

平台相关代码集中在 `hotkey/` 与 `inject/` 的 trait 后面。Windows 已实现：热键为 rdev 低级键盘钩子（默认 Win+Alt 录入 / Ctrl+Alt 修复 / Win+Shift 电脑控制，已避开微信语音输入的 Ctrl+Win），文字注入为剪贴板 + 模拟 Ctrl+V。

## Windows 注意事项

- 默认快捷键为 Win+Alt（录入）/ Ctrl+Alt（修复）/ Win+Shift（指令）；可在设置页勾选「区分左右」，把快捷键精确绑定到左侧或右侧修饰键（如只认右 Win + 右 Alt）。
- App 运行期间，Win 单键、Win+E、Win+R 等系统快捷键保持可用；按住 Win+Alt 录音时开始菜单不会弹出（只有「属于 drop-typing 组合」的 Win 键事件会被拦截）。
- 默认 Win+Shift 与系统截图 Win+Shift+S 存在键位冲突；如需保留系统截图，请把指令通道改为其它组合键。
- 唤醒词模型随 NSIS 安装包分发；若直接运行裸 `drop-typing.exe`，需把项目里的 `src-tauri/models` 目录放在 exe 同目录。唤醒词加载或麦克风监听失败会在暂存条显示黄底红字提示。
- script 动作：macOS 用 zsh（`/bin/zsh -lc`），Windows 默认用 `cmd.exe /C`（动作别名可配置 `shell = "powershell"` 改用 PowerShell 执行单行命令）；`.bat` / `.cmd` / `.ps1` 文件路径按扩展名直接执行，不受 shell 字段影响。

## 热键方案决策

**选用 rdev，不用 tauri-plugin-global-shortcut。** 原因：M1 需要"裸右 ⌘ 单独按下 + 精确 press/release 事件 + 松开时时长判定"，global-shortcut 插件面向组合键按下即触发，拿不到单独的松开事件，也不支持裸修饰键语义。rdev 的 `listen`（CGEventTap）能精确给出 `MetaRight` 的按下/松开，代价是需要辅助功能权限（模拟 Cmd+V 本来也需要，无额外成本）。

## 已知限制（后续里程碑处理）

- 暂存条只读：手动编辑是 M3（当前窗口忽略鼠标事件以保证绝不抢焦点，M3 需换 non-activating NSPanel 方案）
- 剪贴板只按纯文本保存/恢复：若提交前剪贴板里是图片/文件等非文本内容，恢复时会丢失
- 重采样为线性插值，后续可换 rubato
- 光标定位依赖目标 App 的 Accessibility 支持：已做 `AXEnhancedUserInterface` 唤醒（Electron 应用需要），仍有少数 App 取不到插入点，此时依次回退到聚焦元素下方 → 聚焦窗口内底部居中 → 屏幕底部居中
- ASR 上下文偏置（热词/暂存条文本随请求传入）接口已预留，尚未接入
- 无设置界面（M4）、无权限引导流程（M4）

## 用户验证清单

1. `npm install && npm run tauri dev` → 启动后**不显示**暂存条（配置/权限有问题时除外，会黄底红字常显）
2. 授予终端辅助功能权限并重启 → 在文本框中长按右 ⌘，暂存条出现在光标旁，右侧显示"倾听中"+ 红色波形
3. 松开 → 状态依次变为"识别中"→"润色中"，清洗后文本追加到暂存条，保持可见（短按提交后才消失）
4. 在桌面 / Finder（无文本焦点）长按 → 暂存条回退到底部居中
5. 配置 `[llm]` 后长按说带口水话的话（如"嗯那个第一要快第二要稳"）→ 进暂存条的是清洗后文本；切换 `strength = "conservative"` 只加标点
6. 短按右 ⌘ → 文本粘贴到当前聚焦 App，暂存条清空并立即隐藏
7. 拔掉 Key / 断网 → 暂存条黄底红字报错（LLM 清洗失败时降级为原文追加），常显不自动隐藏

## License

MIT，见 [LICENSE](LICENSE)。
