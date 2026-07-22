# drop-typing

> 按住说话、松手出字的语音输入工具（macOS）。当前进度：**M1**。
>
> 开源协议 MIT。产品细节见 [PRD.md](PRD.md)。

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
│   │   ├── pipeline.rs          # 编排：热键 → 录音 → ASR → 暂存条 → 提交
│   │   ├── staging.rs           # 暂存条状态（文本归属 Rust 侧）
│   │   ├── config.rs            # 配置加载（[asr]/[llm] 段 + legacy/env 回退）
│   │   ├── asr/                 # ASR 抽象：批量/实时两套 trait + 每家一个适配器
│   │   │   ├── mod.rs
│   │   │   ├── bailian.rs       #   百炼 qwen3-asr-flash（HTTP，备选）
│   │   │   └── bailian_realtime.rs # 百炼 fun-asr-realtime（WebSocket 流式，默认）
│   │   ├── audio/recorder.rs    # cpal 录音 → 流式 PCM chunk / 整段 WAV
│   │   ├── hotkey/              # 热键抽象：trait HotkeySource（平台相关）
│   │   │   ├── mod.rs
│   │   │   └── macos.rs         #   rdev 全局监听 + 辅助功能权限检测
│   │   └── inject/              # 注入抽象：trait Injector（平台相关）
│   │       ├── mod.rs
│   │       └── macos.rs         #   arboard 剪贴板 + enigo 模拟 Cmd+V
│   ├── examples/test_asr.rs     # ASR 独立手动测试入口
│   └── tauri.conf.json / capabilities/ / Info.plist
├── config.example.toml
└── PRD.md
```

平台相关代码集中在 `hotkey/` 与 `inject/` 的 trait 后面，Windows 移植（PRD：Right Win / Right Alt / Right Shift + Ctrl+V）只需各加一个实现文件。

## 热键方案决策

**选用 rdev，不用 tauri-plugin-global-shortcut。** 原因：M1 需要"裸右 ⌘ 单独按下 + 精确 press/release 事件 + 松开时时长判定"，global-shortcut 插件面向组合键按下即触发，拿不到单独的松开事件，也不支持裸修饰键语义。rdev 的 `listen`（CGEventTap）能精确给出 `MetaRight` 的按下/松开，代价是需要辅助功能权限（模拟 Cmd+V 本来也需要，无额外成本）。

## M1 已知限制（后续里程碑处理）

- 暂存条只读：手动编辑是 M3（当前窗口忽略鼠标事件以保证绝不抢焦点，M3 需换 non-activating NSPanel 方案）
- 剪贴板只按纯文本保存/恢复：若提交前剪贴板里是图片/文件等非文本内容，恢复时会丢失
- 重采样为线性插值，后续可换 rubato
- ASR 上下文偏置（热词/暂存条文本随请求传入）接口已预留，M2 接入
- 无设置界面（M4）、无权限引导流程（M4）

## 用户验证清单

1. `npm install && npm run tauri dev` → 屏幕底部出现暂存条
2. 授予终端辅助功能权限并重启 → 长按右 ⌘ 暂存条出现红色波形
3. 配置好 Key 后长按说话 → 松开后 1-2 秒文本追加到暂存条
4. 短按右 ⌘ → 文本粘贴到当前聚焦 App，暂存条清空并闪绿
5. 拔掉 Key / 断网 → 暂存条黄底红字报错

## License

MIT，见 [LICENSE](LICENSE)。
