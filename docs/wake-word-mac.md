# Mac 端唤醒词方案

> 状态：方案设计完成，待 Phase 1 实施

## 目标

在 drop-typing 的 Rust 后端中集成唤醒词检测，实现完全免提交互。三个功能各自对应一个独立唤醒词，检测到后直接进入对应模式，零歧义。

| 唤醒词 | 通道 | 行为 |
|--------|------|------|
| **DT 打** | 录入 | 说话 → ASR → LLM 清洗 → 追加到暂存条 |
| **DT 修** | 修复 | 说修正指令 → ASR → LLM repair → 替换暂存条 |
| **DT 按** | 指令 | 说按键名 → ASR → 本地词表解析 → 倒计时 → 模拟按键 |

### 命名设计

"DT" 是 drop-typing 的缩写，在中文日常对话中几乎零出现，天然低误触。后面跟一个单音节动词，三个音节说完，比 "Hey Drop" 更短：

| 唤醒词 | 音节数 | 含义 |
|--------|--------|------|
| DT 打 | 3（dì tī dǎ） | 打字/录入 |
| DT 修 | 3（dì tī xiū） | 修改/修复 |
| DT 按 | 3（dì tī àn） | 按键/快捷键 |

**设计原则**：一个唤醒词 = 一个模式，一一对应。用户不需要在唤醒词后再说标记词（如"DT 打"后直接说内容即可），pipeline 也知道当前是哪个模式——唤醒词检测结果本身就是 `RecordMode`。

分两阶段推进：
- **Phase 1**：唤醒词引擎直接集成，持续监听，三个唤醒词 → 三个模式自动路由
- **Phase 2**：引入 VAD（Voice Activity Detection）作为前置过滤器，降低空闲时的 CPU 开销

## 为什么集成进 drop-typing 而非独立进程

独立进程（Python 守护进程 → CGEvent 发 F13）做验证很快，但集成到 Rust 后端有三个不可替代的收益：

### 1. 共享音频流 —— 核心收益

```
独立进程方案：两条互不知晓的流

  Python 进程                      drop-typing
  pyaudio 开 mic                   cpal 开 mic（收到 F13 后才开）
     │                                │
     │ 检测到唤醒词 → CGEvent F13 ───→│ 开流。
     │                                │ ← 冲突：两个进程抢同一个设备
     │                                │   且"帮我写邮件"的前几个字已丢失

集成方案：一条流

  cpal 开 mic（唯一一次）
     │
     ▼
  Ring Buffer（环形缓冲区，保留最近 3 秒音频）
     │
     ├─→ 唤醒词模型（持续推理，多输出头）
     │     │ 检测到 "DT 打" → WakeEvent { word: Da }
     │     │ 检测到 "DT 修" → WakeEvent { word: Xiu }
     │     │ 检测到 "DT 按" → WakeEvent { word: An }
     │     ▼
     └─→ 裁掉唤醒词部分 → 后续音频直接送入 ASR
           ↑
          缓冲区音频帧连续，唤醒词前后的语音都不丢
```

### 2. 零衔接延迟

唤醒词和 ASR 在同一个进程、同一段缓冲区里接力——唤醒词检测到的一瞬间，前面的音频帧已经在内存里了，裁掉 "Hey Drop" 部分直接喂给 ASR，用户的后续语音一个字不丢。

### 3. 统一配置和错误反馈

唤醒词开关、灵敏度、自定义唤醒词——全在 `~/.drop-typing.toml` 里。唤醒词引擎挂了，用暂存条黄底红字报错，用户能看见，而不是守护进程静默退出然后用户疑惑"怎么不灵了"。

---

## Phase 1：唤醒词引擎直接集成

### 1.1 技术选型

| 组件 | 选型 | 理由 |
|------|------|------|
| 唤醒词引擎 | **openWakeWord** | Apache 2.0 开源，可自定义唤醒词，社区活跃 |
| 模型格式 | **ONNX** | 开放标准，不绑定框架 |
| 推理运行时 | **ort**（ONNX Runtime） | Rust binding 成熟（`ort` crate），CPU 推理性能好 |
| 音频输入 | **cpal**（现有依赖） | 与 `audio/recorder.rs` 共享，不引入新依赖 |

openWakeWord 模型规格：
- 输入：80ms 音频帧（16kHz 单声道 16bit PCM → 1280 采样点）
- 输出：**三个**置信度分数（0.0–1.0），分别对应 "DT 打"、"DT 修"、"DT 按"
- 模型大小：约 400KB（三个唤醒词共享同一个特征提取器，仅输出头不同）
- 推理耗时：< 1ms / 帧（Apple Silicon，三路输出一次性计算）

### 1.2 新增依赖

```toml
# src-tauri/Cargo.toml
[dependencies]
ort = "2"              # ONNX Runtime Rust binding
```

`ort` crate 会在 build 时下载对应平台的 ONNX Runtime 动态库（约 8MB），打包进 `.app` bundle。

### 1.3 新增模块

```
src-tauri/src/
├── wakeword/
│   ├── mod.rs           # WakeWordEngine trait + 检测器状态机
│   ├── openwakeword.rs  # openWakeWord ONNX 模型加载 + 推理
│   └── models/          # 预训练模型文件（构建时 embed）
│       └── dt_wake_words.onnx
├── audio/
│   ├── recorder.rs      # 现有：cpal 录音（按需开流）
│   └── listener.rs      # 新增：cpal 持续监听（始终开流）
├── pipeline.rs          # 改动：新增 Listening 状态
└── config.rs            # 改动：新增 [wakeword] 配置段
```

### 1.4 架构

```
┌─────────────────────────────────────────────────────┐
│                  audio/listener.rs                   │
│                                                     │
│  cpal 持续开流（16kHz 16bit 单声道）                  │
│     │                                               │
│     ▼                                               │
│  Ring Buffer（环形缓冲区，3 秒 = 96000 采样）          │
│     │                                               │
│     ├─→ wakeword/openwakeword.rs                    │
│     │   每 80ms 取一帧（1280 采样）                    │
│     │   ort 推理 → 唤醒词置信度（小易记/小易修/小易小易） │
│     │   任一置信度 > 阈值 → WakeEvent::Detected { word }│
│     │                                               │
│     └─→ pipeline.rs                                 │
│         收到 WakeEvent::Detected { word }            │
│         word 直接映射到 RecordMode：                  │
│           Da  → RecordMode::Input                    │
│           Xiu → RecordMode::Repair                   │
│           An  → RecordMode::Command                  │
│         从 Ring Buffer 中取"唤醒词之前 0.5s + 之后"    │
│         裁掉唤醒词部分 → 喂给 ASR                      │
│                                                     │
│  ┌─────────────────────────────────────────┐        │
│  │  时序示意                                │        │
│  │                                         │        │
│  │  [...静音...][DT 打][帮我写一封邮件...]    │        │
│  │              ↑                          │        │
│  │         唤醒词检测                       │        │
│  │              │                          │        │
│  │             Ring Buffer 里已有完整音频    │        │
│  │             裁掉 "DT 打"（约 0.5s）       │        │
│  │             送入 ASR: "帮我写一封邮件"    │        │
│  └─────────────────────────────────────────┘        │
└─────────────────────────────────────────────────────┘
```

### 1.5 唤醒词 → RecordMode 映射

唤醒词检测输出直接决定模式，不需要额外分类器或 ASR 规则：

```rust
// wakeword/mod.rs
pub enum WakeWord {
    Da,   // "DT 打" → 录入
    Xiu,  // "DT 修" → 修复
    An,   // "DT 按" → 指令
}

impl WakeWord {
    pub fn to_record_mode(self) -> RecordMode {
        match self {
            WakeWord::Da  => RecordMode::Input,
            WakeWord::Xiu => RecordMode::Repair,
            WakeWord::An  => RecordMode::Command,
        }
    }

    /// 每个唤醒词有不同的语音时长，用于裁切 Ring Buffer
    pub fn duration_ms(self) -> u64 {
        match self {
            WakeWord::Da  => 500,  // "DT 打" 约 0.5s
            WakeWord::Xiu => 550,  // "DT 修" 约 0.55s
            WakeWord::An  => 450,  // "DT 按" 约 0.45s
        }
    }
}
```

### 1.6 Ring Buffer 设计

唤醒词检测到之后的处理需要"倒带"——唤醒词前面的音频不能丢，唤醒词之后的音频也在同一段缓冲区里。一个环形缓冲区自然满足这个需求。

```
Ring Buffer（3 秒 = 96000 个 i16 采样 = 192KB）

  write_ptr ──→  [最新的 80ms 帧刚写入这里]
                  │
  read_ptr ────→  [唤醒词检测到的那一刻，从这里开始取]
                  │
                  ├─ pre_roll:  唤醒词前 0.5s（16000 采样）→ 保留上下文
                  ├─ wake_word: 唤醒词本身（~0.5s）→ 丢弃
                  └─ post_roll: 唤醒词后直到静音 → 送入 ASR
```

关键参数可配置：

| 参数 | 默认值 | 含义 |
|------|--------|------|
| `ring_buffer_duration_ms` | 3000 | 环形缓冲区时长 |
| `pre_roll_ms` | 500 | 唤醒词前保留多少音频 |
| `wake_word_duration_ms` | 600 | 唤醒词自身估计时长（用于裁切） |

### 1.7 Pipeline 状态机改动

现有状态机（Idle / Recording / PendingCommit）需新增一个 **Listening** 态：

```
现有状态机：
  Idle ──TriggerDown──▶ Recording ──TriggerUp──▶ 分发 ──▶ Idle / PendingCommit

唤醒词状态机：
  Idle ──▶ Listening（持续监听，三路唤醒词推理）
              │
              │ WakeEvent::Detected { word }
              ▼
           Recording（自动进入，mode = word.to_record_mode()）
              │
              │ 静音 > 1.5s 或 用户按 F14
              ▼
           分发（ASR → 按 mode 分发 → 清洗/修复/指令执行）
              │
              ▼
           Listening（回到持续监听）

路由对照：
  DT 打 ──▶ RecordMode::Input   ──▶ clean_and_append()
  DT 修 ──▶ RecordMode::Repair  ──▶ repair_and_replace()
  DT 按 ──▶ RecordMode::Command ──▶ run_command()
```

改动要点：

- **Listening** 态是后台态——暂存条隐藏，后台持续推理三个唤醒词
- 唤醒词检测到后，`WakeWord` 直接映射为 `RecordMode`，自动从 Listening → Recording
- **pipeline 的现有模式分发逻辑零改动**——`RecordMode::Input/Repair/Command` 的后续流程（ASR → LLM → 暂存条 / run_command）完全不变
- 录音结束条件：检测到静音 > `silence_timeout_ms`，或用户按 F14 提交
- 唤醒词启用时，物理热键（右 ⌘/右 ⌥/右 Shift）仍然可用——热键和唤醒词共存不互斥
- 唤醒词检测到后，暂存条弹出，状态徽章显示唤醒词名（"DT 打 ✓" / "DT 修 ✓" / "DT 按 ✓"），然后切换为对应模式的状态（"识别中" / "修复识别中" / "指令识别中"）

### 1.8 配置设计

```toml
# ~/.drop-typing.toml

[wakeword]
enabled = true                    # 是否开启唤醒词（默认 false）
model = "dt_wake_words"           # 模型名（对应 models/dt_wake_words.onnx）
silence_timeout_ms = 1500         # 唤醒后多久静音判定录音结束
pre_roll_ms = 500                 # 唤醒词前保留的音频时长

# 每个唤醒词可独立调整灵敏度（可选，都有默认值）
[wakeword.sensitivity]
da = 0.5                          # "DT 打" 检测阈值（越高越难误触，但越难唤醒）
xiu = 0.5                         # "DT 修" 检测阈值
an = 0.5                          # "DT 按" 检测阈值（建议稍高于其他两者，
                                  #   因指令通道有安全风险）
```

`enabled = false` 时不加载唤醒词模型，不创建 listener 流，行为和现在完全一致。

### 1.9 功耗与性能

| 项目 | 数值 |
|------|------|
| cpal 持续流（16kHz 16bit 单声道） | CPU < 0.5%（仅 PCM 拷贝，无编码） |
| ONNX 推理（每 80ms 一帧，三路输出） | CPU < 1%（模型 ~400KB，Apple Silicon 推理 < 1ms/帧） |
| 内存（Ring Buffer + 模型） | < 5MB |
| **总空闲开销** | **CPU < 1.5%，内存 < 5MB** |

对 MacBook 续航的影响可以忽略不计。

### 1.10 误触风险分析

三个唤醒词共享 "DT" 前缀，"DT" 在中文对话中几乎零出现（不同于 "Hey""OK""Hi" 等常用词）。单模型三路输出的设计进一步降低了误触——三路置信度相互独立，日常语音中三个通道的置信度在随机噪音下都低于阈值。

```
日常对话:
  "D" + "T"         → 置信度 < 0.05（单个辅音不是完整词）
  "打"              → 置信度 < 0.1（前缀缺失）
  "DT 打"            → 置信度 > 0.8 ✅
  "DT 好"            → 三路分别检测，都 < 0.1 ✅ 不触发
  "好的"             → 三路 < 0.1 ✅ 不触发
```

### 1.11 自定义唤醒词训练

openWakeWord 支持单模型多唤醒词。训练流程：

1. 录制样本：
   - "DT 打" × 50–100 条（不同语气、距离、背景噪音）
   - "DT 修" × 50–100 条
   - "DT 按" × 50–100 条
   - 负样本 × 200 条（日常对话、其他常见语音指令、静音/环境噪音）
2. 用 openWakeWord 训练脚本微调，输出三路的多唤醒词模型
3. 输出 `dt_wake_words.onnx`，放到 `src-tauri/src/wakeword/models/`
4. 构建时 `include_bytes!` 嵌入二进制

训练脚本（Python，一次性离线操作，不进入 drop-typing 仓库）：
```bash
pip install openwakeword
python -m openwakeword.train \
    --keywords "dt_da" "dt_xiu" "dt_an" \
    --pos-samples ./recordings/dt_da/ ./recordings/dt_xiu/ ./recordings/dt_an/ \
    --neg-samples ./recordings/background/ \
    --output dt_wake_words.onnx
```

---

## Phase 2：VAD 前置过滤

### 2.1 为什么需要 VAD

Phase 1 的唤醒词模型虽然单次推理很快（< 1ms），但**每 80ms 就推理一次**，全年无休。虽然 CPU 开销不大，但有一种更高效的方式。

**VAD（Voice Activity Detection）**只做一件事：判断当前音频帧是"有人说话"还是"静音"。VAD 模型极小（几 KB），推理极快（微秒级）。用 VAD 做第一级过滤，只有检测到语音活动时才喂给唤醒词模型：

```
Phase 1（无 VAD）：
  音频帧 → 唤醒词模型（每 80ms，1ms/次）→ 全年无休
  开销：12.5 次推理/秒

Phase 2（有 VAD）：
  音频帧 → VAD（每 20ms，几 μs/次）→ 检测到语音 → 才跑唤醒词模型
  开销：VAD 常年跑（极低），唤醒词模型仅在有声音时跑
  节省：静音时段唤醒词模型完全休眠
```

日常使用中，绝大部分时间是静音——VAD 能在静音期间让唤醒词推理完全空闲。

### 2.2 技术选型

| 组件 | 选型 | 理由 |
|------|------|------|
| VAD 引擎 | **WebRTC VAD** | C 库，Rust binding 成熟（`webrtc-vad` crate），几 KB 大小 |
| 帧长 | 20ms（320 采样 @ 16kHz） | WebRTC VAD 标准帧长 |
| 输出 | 0（静音/噪音）或 1（语音） | 二元判定 |

`webrtc-vad` crate 包含预编译的 WebRTC VAD C 代码，不需要 ONNX Runtime 推理链。

### 2.3 VAD + 唤醒词 两级流水线

```
┌───────────────────────────────────────────────────┐
│                 Two-Stage Wake Pipeline           │
│                                                   │
│  cpal 持续流（16kHz）                              │
│     │                                             │
│     ▼                                             │
│  Ring Buffer（3 秒，和 Phase 1 共用）              │
│     │                                             │
│     ▼                                             │
│  ┌─────────────────────┐                          │
│  │ Stage 1: WebRTC VAD │  每 20ms 一帧             │
│  │ 模型: ~5KB          │  推理: ~2μs/帧            │
│  │ 输出: 0 (静音) 或 1 (语音)                      │
│  └──────┬──────────────┘                          │
│         │                                         │
│         │ 输出 = 1（检测到语音）                     │
│         ▼                                         │
│  ┌─────────────────────┐                          │
│  │ Stage 2: Wake Word  │  仅在 Stage 1 触发后运行   │
│  │ 模型: ONNX ~400KB   │  推理: ~1ms/帧            │
│  │ 输出: 三置信度       │                          │
│  └──────┬──────────────┘                          │
│         │                                         │
│         │ 置信度 > sensitivity → WakeEvent::Detected { word }│
│         ▼                                         │
│  pipeline.rs → Recording                          │
└───────────────────────────────────────────────────┘
```

### 2.4 VAD 状态的 Hysteresis（迟滞）

VAD 的语音/静音判定需要迟滞逻辑，避免短停顿导致状态抖动：

```
状态：SILENCE
  │
  │ 连续 N 帧 VAD = 1（如 5 帧 = 100ms）
  ▼
状态：VOICE_ACTIVE
  │ wake word 模型开始推理
  │
  │ 连续 M 帧 VAD = 0（如 30 帧 = 600ms）
  ▼
状态：SILENCE
  │ wake word 模型停止推理
```

| 参数 | 默认值 | 含义 |
|------|--------|------|
| `vad_speech_trigger_frames` | 5 | 连续多少帧语音才进入 VOICE_ACTIVE |
| `vad_silence_trigger_frames` | 30 | 连续多少帧静音才回到 SILENCE |

### 2.5 CPU 开销对比

假设日常使用中 10% 时间有人在说话（实际更低）：

| 方案 | 唤醒词模型推理频率 | CPU（空闲） | CPU（说话时） |
|------|-------------------|-------------|--------------|
| Phase 1（无 VAD，三路推理） | 12.5 次/秒 | ~1% | ~1% |
| Phase 2（有 VAD） | 仅在 VOICE_ACTIVE 时 | < 0.1% | ~1% |
| 无唤醒词（当前） | 无 | 0% | 0% |

Phase 2 让空闲功耗降至接近零——VAD 本身的推理几乎不占 CPU（μs 级别），而唤醒词模型只在真正有声音时才启动。

---

## ⚠️ 橙色指示灯问题

### 问题描述

macOS Ventura 起，任何 App 使用麦克风时，菜单栏右侧出现橙色圆点。cppal 持续开流，橙点就永远不灭。

```
🔴 正使用麦克风："drop-typing"
```

这是 **macOS 系统级硬约束**——只要音频输入流在运行，系统就亮灯。无法绕过。VAD 也解决不了——VAD 只是降低 CPU 推理开销，但不关麦克风流。

### 用户感知

- 橙点全天亮着 → 用户觉得被持续窃听 → 不安
- "不是说只在本地推理不上传吗？"——你解释了，但橙点是大环境教育的信号，用户的第一反应不是读你的解释

### 三种方案的橙点行为

| 方案 | 橙点行为 | 用户感受 |
|------|---------|---------|
| **当前方案**（右 ⌘ 热键） | 仅在录音时亮 | ✅ 正常——说话时才亮 |
| **软件唤醒词**（Phase 1/2） | 永远亮着 | ❌ 用户不安 |
| **硬件唤醒词**（DSP + USB HID） | 仅在录音时亮 | ✅ 正常——和热键一样 |

### 这个矛盾意味着什么

软件唤醒词方案的价值不在于"长期用"，而在于：

1. **快速验证唤醒词语音交互的体验**——橙点问题可以暂时接受，验证周期 1–2 周
2. **确认唤醒词作为交互范式的价值**——比按键好多少？误触率能接受吗？
3. **为硬件方案提供确定性需求**——如果软件方案验证了 "这个交互真好"，那就值得投入硬件 DSP 版本来解决橙点问题

### Phase 2 的定位

VAD 让软件方案更高效了，但它不能解决橙点问题。Phase 2 的真正价值在于：**当你正在用软件方案验证交互时，VAD 降低了功耗**。它延长了验证阶段的可用性，但长期方案仍然是硬件的 DSP 唤醒词。

---

## Phase 3：说话人声纹校验

### 3.1 什么问题

Phase 1/2 的通用唤醒词模型存在一个缺陷：**它只关心"说了什么"，不关心"谁说的"。** 在多人场景或嘈杂环境下，非用户的语音也可能触发唤醒词。

- 办公室里同事喊了一句 "DT 打" → 你的电脑被唤醒
- 视频会议里有人说 "DT 修" → 误触
- 电视/播客里恰好出现 "DT" 音 → 概率极低，但非零

Phase 3 不重新训练唤醒词模型，而是**在通用模型的输出上加一层声纹校验**——只响应登记用户的声音。

### 3.2 原理：不是训练，是校验

```
Sound Event      Neural Network Training      Speaker Verification
─────────────────────────────────────────────────────────────────
做什么            重新学习音频→文字的映射        计算"这个声音像不像你"
需要什么          完整训练框架（PyTorch）       一个现成的声纹模型
                                    + 你的 3-5 条样本
计算量            数小时（GPU）               < 0.1ms（CPU）
用户等待          不可接受                     即时
模型大小变化      整模型更新（~400KB）          仅声纹模板（几百字节）
在哪里做          云端                         本地
```

**Speaker Verification 不做训练——它提取一个数学特征然后比相似度。**

### 3.3 技术选型

| 组件 | 选型 | 理由 |
|------|------|------|
| 声纹提取模型 | **WeSpeaker**（ONNX） | 开源（Apache 2.0），中文场景最优，模型 ~30MB |
| 相似度计算 | 余弦相似度 | 纯数学运算，不依赖推理框架 |
| 声纹模板存储 | `~/.drop-typing.toml` | 每个唤醒词一个 256 维浮点向量（约 1KB base64） |

WeSpeaker 是西北工业大学开源的中文说话人识别模型，在中文语音上的声纹区分度显著优于英文模型（如 ECAPA-TDNN）。

### 3.4 架构

```
音频帧 → Ring Buffer
              │
              ▼
    ┌─────────────────────┐
    │ Stage 1: WebRTC VAD │  （Phase 2）
    └──────┬──────────────┘
           │ VOICE_ACTIVE
           ▼
    ┌─────────────────────┐
    │ Stage 2: Wake Word  │  通用唤醒词模型（Phase 1）
    │ "DT 打" 置信度 0.82  │
    └──────┬──────────────┘
           │ 置信度 > sensitivity
           ▼
    ┌─────────────────────────┐
    │ Stage 3: Speaker Verify │  声纹校验（Phase 3，新增）
    │                         │
    │ 1. 当前音频帧            │
    │    → WeSpeaker 提取嵌入  │
    │    → 256维向量 v_curr   │
    │                         │
    │ 2. cos_sim(v_curr,      │
    │           v_enrolled)   │
    │    = 0.91               │
    │                         │
    │ 3. 0.91 > threshold?    │
    │    Yes → 置信度 × 1.2   │
    │    → WakeEvent          │
    └─────────────────────────┘
```

### 3.5 录入流程

用户在设置界面中为每个唤醒词录入声纹：

```
┌─────────────────────────────────────────┐
│  唤醒词声纹录入                           │
│                                         │
│  [DT 打]  请用正常语气说第 1 遍           │
│  ████████████████░░░░ 录音中             │
│                                         │
│  已录入: ██░░░ 2/5                       │
│                                         │
│  录制完成后自动提取声纹向量 ✅             │
│                                         │
│  ─────────────────────────────────      │
│                                         │
│  [DT 修]  未录入 (可选)                   │
│  [DT 按]  未录入 (可选)                   │
│                                         │
│  💡 录入后仅响应你的声音，                    │
│     未录入的唤醒词使用通用模式                │
└─────────────────────────────────────────┘
```

录入交互细节：

1. 用户点击"录入声纹"，选择一个唤醒词
2. 倒计时 3-2-1，每次录制 1.5 秒音频
3. 实时检测是否检测到声音、是否过短/过长
4. 录满 3-5 条后，计算声纹嵌入向量的均值
5. 用户可以选择只用 3 条（快但不够稳）或 5 条（更稳健）
6. 录入完成后给一个"相似度一致性"的反馈（5 条彼此是否足够相似）

### 3.6 声纹校验的决策逻辑

```rust
// 伪代码
fn verify_speaker(audio_frame: &[i16], enrolled: &[f32; 256]) -> f32 {
    // 1. 提取当前帧的声纹嵌入向量
    let embedding: [f32; 256] = wespeaker_model.extract(audio_frame);

    // 2. 计算余弦相似度
    let similarity = cosine_similarity(&embedding, enrolled);

    // 3. 根据相似度返回缩放因子
    if similarity > 0.75 {
        1.2   // 确认是你的声音 → boost 置信度
    } else if similarity > 0.6 {
        0.8   // 不太确定     → 轻微抑制
    } else {
        0.3   // 明显不是你的声音 → 强力抑制
    }
}

// 最终是否触发：
// wake_word_confidence × verify_scale > sensitivity → 触发
```

关键设计决策：

- **声纹校验不是硬门禁**——它不是"不匹配就拒绝"，而是"匹配就 boost，不匹配就抑制"。因为这个模型本身也会出错（感冒、环境噪音）
- **未录入的唤醒词不受影响**——用户可以选择只给 DT 按（指令通道，安全关键）录声纹，DT 打和 DT 修仍然是通用模式
- **多声纹共存**——如果设备被多人使用，可以登记多个声纹模板

### 3.7 配置设计

```toml
# ~/.drop-typing.toml

[wakeword.voiceprint]
enabled = true                           # 是否开启声纹校验（默认 false）
verify_threshold = 0.7                   # 余弦相似度阈值
boost_scale = 1.2                        # 匹配时的置信度放大倍数
suppress_scale = 0.3                     # 不匹配时的抑制倍数

[wakeword.voiceprint.enrolled]
# 每个唤醒词可单独登记声纹，未登记的不做声纹校验
da = "AeQ9x3...(base64 编码的 256 维向量)"
xiu = "BfR2k7..."
# an 未登记 → 通用模式
```

### 3.8 性能开销

| 项目 | 数值 |
|------|------|
| WeSpeaker ONNX 模型大小 | ~30MB（加载到内存后约 35MB） |
| 单次声纹提取 | < 0.5ms（Apple Silicon） |
| 余弦相似度计算 | < 0.001ms（纯数学） |
| 声纹模板存储 | ~1KB / 唤醒词 |
| Phase 3 新增内存 | ~35MB |
| Phase 3 新增 CPU | < 0.1%（仅在 VAD 激活时跑） |

声纹校验只在 VAD 检测到语音 + 唤醒词置信度超过基础阈值后才执行——不是在每一帧 80ms 上都跑。实际触发频率极低（一天可能就十几次），所以 30MB 的 WeSpeaker 模型虽然大，但几乎不耗 CPU。

### 3.9 局限性

| 场景 | 效果 |
|------|------|
| 你在安静房间说 DT 打 | ✅ boost，更容易唤醒 |
| 同事在旁说 DT 打 | ✅ 显著抑制 |
| 你感冒嗓子沙哑 | ⚠️ 声纹偏移，可能需要重新录入 |
| 你用很夸张的语气说 DT 打 | ⚠️ 可能与登记时的声纹差距较大 |
| 你戴着口罩说话 | ✅ 基本不受影响 |
| 背景有强噪音 | ⚠️ 声纹提取质量下降 |

声纹不是万能药——它的核心价值是**把你的 DT 打和别人的 DT 打区分开**，让通用模型的误触率大幅降低。但它不能替代好的唤醒词设计（DT 前缀本身低误触才是第一道防线）。

---

## 与整体方案的关联

```
                     ┌─────────────────────┐
                     │   软件唤醒词方案      │
                     │   (本文档)           │
                     │                     │
  USB 硬件按键       │ Phase 1: 唤醒词集成   │
  (usb-mic-         │ Phase 2: VAD 前置    │
   hardware.md)      │ Phase 3: 声纹校验    │
     │               │                     │
     │               │ 短期验证用            │
     │               │ 橙点常亮             │
     │               └──────────┬──────────┘
     │                          │
     │                    验证交互范式
     │                    确认值得硬件化
     │                          │
     └──────────┬───────────────┘
                │
                ▼
     ┌─────────────────────┐
     │   硬件 DSP 唤醒词     │
     │   (终极方案)          │
     │                     │
     │ DSP 持续监听唤醒词    │
     │ 检测到 → HID 按键    │
     │ Mac 侧仅录音时亮橙点  │
     └─────────────────────┘
```

- **USB 硬件按键** → 解决"不用键盘按键"的问题，橙色灯体验好，但不能免提
- **软件唤醒词** → 解决"完全免提"的问题，验证交互范式，但橙色灯常亮
- **硬件 DSP 唤醒词** → 同时解决"免提"和"橙点"问题，但需要硬件投入

三者不是竞争关系，是递进关系。USB 按键先做（硬件最简单），软件唤醒词并行验证（成本为零），确认无误后合流为硬件 DSP 方案。

---

## 实施顺序建议

```
Step 1: F13 热键支持（现有 pipeline 改动，~50 行，半天）
        │  产出：pipeline 能响应 F13/F14/F15
        │
Step 2: Phase 1 唤醒词集成（新增 wakeword/ 模块，~500 行，3–5 天）
        │  产出：说 "DT 打" 录入、"DT 修" 修复、"DT 按" 指令
        │  注意：橙点常亮，仅用于验证
        │
Step 3: Phase 2 VAD 前置（新增 vad 过滤逻辑，~200 行，2 天）
        │  产出：静音时段唤醒词模型休眠，空闲 CPU → 零
        │
Step 4: Phase 3 声纹校验（新增 voiceprint/ 模块，~300 行，3 天）
        │  产出：设置界面录入声纹，仅响应登记用户的声音
        │
Step 5: 同时推进 USB 硬件（按键版）和软件唤醒词验证
        │  用 1–2 周确认三种交互（按键/唤醒词/声纹）各自优劣
        │
Step 6: 合流 → 硬件 DSP 唤醒词（DSP 芯片 + ESP32 + USB HID）
```

---

## 新增文件清单

| 文件 | 用途 | 预估行数 |
|------|------|---------|
| `src-tauri/src/wakeword/mod.rs` | `WakeWordEngine` trait + 检测器状态机 | ~100 |
| `src-tauri/src/wakeword/openwakeword.rs` | openWakeWord ONNX 加载 + 推理 | ~200 |
| `src-tauri/src/wakeword/models/dt_wake_words.onnx` | 三路唤醒词模型（embed） | 二进制 |
| `src-tauri/src/audio/listener.rs` | cpal 持续监听 + Ring Buffer | ~150 |
| `src-tauri/src/audio/vad.rs` | WebRTC VAD 封装（Phase 2） | ~80 |
| `src-tauri/src/voiceprint/mod.rs` | 声纹校验 trait + 登记/验证逻辑（Phase 3） | ~150 |
| `src-tauri/src/voiceprint/wespeaker.rs` | WeSpeaker ONNX 加载 + 声纹提取（Phase 3） | ~120 |

## 改动文件清单

| 文件 | 改动 | 预估行数 |
|------|------|---------|
| `src-tauri/src/pipeline.rs` | 新增 Listening 状态、唤醒词事件处理 | ~150 |
| `src-tauri/src/config.rs` | 新增 `[wakeword]` 配置段 | ~40 |
| `src-tauri/Cargo.toml` | 新增 `ort`、`webrtc-vad` 依赖 | ~5 |
