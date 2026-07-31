# 唤醒词引擎迁移方案：openWakeWord → sherpa-onnx

> 状态：方案设计完成，待 review

## 一、动机

当前唤醒词实现基于 openWakeWord，自行维护 Mel 特征提取（`mel.rs`）、ONNX 推理（`openwakeword.rs`）、以及 WakeWordEngine trait。存在以下痛点：

1. **openWakeWord 已停滞** —— 上游 openWakeWord 社区活跃度下降，预训练模型有限（仅少量英文唤醒词），无官方中文模型
2. **自行维护 Mel 特征提取** —— `mel.rs` 约 270 行手工 FFT + mel filterbank + reflect padding 代码，与 torchaudio 的对齐靠人工验证，出问题难排查
3. **没有 VAD** —— 当前方案每 80ms 推理一次，全年无休；虽然 CPU 开销低，但缺少 VAD 前置过滤
4. **模型训练流程割裂** —— 训练在 Python openWakeWord 仓库做、推理在 Rust ort crate 做，两边特征提取参数需人工对齐

**sherpa-onnx** 是 k2-fsa 维护的开源语音工具包（Apache 2.0），提供开箱即用的 Keyword Spotter API，特征提取内建、支持流式推理、有预训练中英文 KWS 模型、内置 VAD。用 sherpa-onnx 替换 openWakeWord 可大幅简化代码、获得更好的模型生态。

## 二、sherpa-onnx 关键信息

| 项目 | 详情 |
|------|------|
| 仓库 | https://github.com/k2-fsa/sherpa-onnx |
| License | Apache 2.0 |
| Rust crate | `sherpa-onnx = "1.13"`（官方维护，非社区绑定） |
| 最新版 | v1.13.3（2026-06-16），onnxruntime 1.26.0 |
| macOS 支持 | x64 + arm64 均有预编译库，首次 build 自动下载 |
| 推理后端 | ONNX Runtime（CPU），可选 DirectML/CUDA/CoreML |
| KWS 模型 | Zipformer Transducer，约 3.3M 参数，RTF < 0.04 |
| 中文模型 | `sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01`（WenetSpeech 10000h 训练） |
| 英文模型 | `sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01`（Gigaspeech 训练） |
| 双语模型 | `sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20`（中英混合） |
| 特征提取 | 内建（无需自行 FFT/mel filterbank） |
| API 风格 | 流式：`create_stream()` → `accept_waveform()` → `decode()` → `get_result()` |

### sherpa-onnx KeywordSpotter 核心 API

```rust
use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig};

// 1. 配置
let mut config = KeywordSpotterConfig::default();
config.model_config.transducer.encoder = Some("./encoder.onnx".into());
config.model_config.transducer.decoder = Some("./decoder.onnx".into());
config.model_config.transducer.joiner = Some("./joiner.onnx".into());
config.model_config.tokens = Some("./tokens.txt".into());
config.keywords_file = Some("./keywords.txt".into());    // 自定义唤醒词
config.keywords_threshold = 0.25;                         // 检测阈值
config.keywords_score = 1.0;                              // 唤醒词 boost

// 2. 创建 spotter
let kws = KeywordSpotter::create(&config)?;

// 3. 流式推理
let stream = kws.create_stream();                         // 或 create_stream_with_keywords()
stream.accept_waveform(16000, &audio_samples);
while kws.is_ready(&stream) {
    kws.decode(&stream);
}
let result = kws.get_result(&stream);                     // Option<KeywordResult>
// result.keyword: String    — 检测到的唤醒词
// result.tokens: String     — token 序列
// result.timestamps: Vec<f32> — 时间戳
// result.start_time: f32    — 起始时间

// 4. 重置/复用
kws.reset(&stream);
```

### keywords.txt 格式

```
▁DT ▁打
▁DT ▁修
▁DT ▁按
```

需要用 `sherpa-onnx-cli text2token` 工具把自然语言关键词转为模型 token 格式。

## 三、预训练模型选择

### 推荐：中英双语模型

**`sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20`**

- 2025 年 12 月发布，最新双语模型
- 同时支持中文和英文唤醒词
- 约 3M 参数，RTF 约 0.036（Apple Silicon 上 < 1ms/帧）
- 下载地址：`https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20.tar.bz2`

### 备选：分别用中英文单语模型

- 中文：`sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01`
- 英文：`sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01`
- 可按 `[[wakeword.model]]` 分别加载

### 模型文件结构

每个模型目录包含 4 个文件：
```
encoder.onnx       # Zipformer encoder（~2.5MB）
decoder.onnx       # Zipformer decoder（~0.5MB）
joiner.onnx        # Transducer joiner（~0.3MB）
tokens.txt         # 词表
```

总大小约 3.3MB，比当前 openWakeWord 模型（~400KB）大约 8 倍，但仍然很小。

## 四、影响范围分析

### 4.1 可删除的代码

| 文件 | 行数 | 说明 |
|------|------|------|
| `src-tauri/src/wakeword/mel.rs` | ~275 | Mel 特征提取器——sherpa-onnx 内建 |
| `src-tauri/src/wakeword/openwakeword.rs` | ~288 | openWakeWord ONNX 推理——替换为 sherpa-onnx KeywordSpotter |
| `src-tauri/src/wakeword/mod.rs` 中的 `WakeWordEngine` trait | ~15 | 不再需要 trait 抽象 |
| `src-tauri/src/wakeword/mod.rs` 中的 `NoopEngine` | ~8 | 用 `Option<KeywordSpotter>` 替代 |
| `src-tauri/src/wakeword/mod.rs` 中的 `CompositeEngine` | ~60 | sherpa-onnx 原生支持多唤醒词 |
| **合计** | **~646** | |

### 4.2 需新增的代码

| 文件 | 行数 | 说明 |
|------|------|------|
| `src-tauri/src/wakeword/sherpa.rs` | ~200 | sherpa-onnx KeywordSpotter 封装（加载、流管理、结果解析） |
| `src-tauri/src/wakeword/mod.rs`（重写） | ~120 | 工厂函数 + 类型定义（WakeWord/WakeEvent 保留） |
| **合计** | **~320** | |

### 4.3 需修改的代码

| 文件 | 改动 | 行数 |
|------|------|------|
| `src-tauri/Cargo.toml` | 移除 `ort`、`rustfft`；新增 `sherpa-onnx` | ~5 |
| `src-tauri/src/audio/listener.rs` | `start_wake_word()` 改为 sherpa-onnx 流式接口 | ~80 |
| `src-tauri/src/pipeline.rs` | 适配新的 WakeEvent（带 start_time 时间戳），其余逻辑基本不变 | ~30 |
| `src-tauri/src/config.rs` | 简化 wakeword 配置：不再区分 multi/single，统一为 keywords 列表 | ~50 |
| `config.example.toml` | 更新 [wakeword] 配置示例 | ~20 |

### 4.4 不受影响的部分

- **Pipeline 状态机**：Listening / Recording / Idle / PendingCommit 不变
- **ASR 链路**：唤醒词触发后 TailReader → PCM → ASR 完全不变
- **LLM 清洗链路**：`clean_and_append` / `repair_and_replace` / `run_command` 不变
- **暂存条前端**：全部不变
- **热键系统**：与唤醒词共存逻辑不变
- **RingBuffer + ContinuousListener**：保留（sherpa-onnx 需要 PCM 输入）
- **macOS 橙色指示灯约束**：不变（持续开流仍然亮灯）

## 五、架构变更

### 5.1 当前架构

```
ContinuousListener → RingBuffer (f32 PCM)
                          │
                    wake word thread (每 80ms 读 1280 采样)
                          │
              ┌───────────┴───────────┐
              │   WakeWordEngine trait │
              │   ├─ OpenWakeWord      │  ← multi 模式：自训模型 + ort 推理
              │   ├─ SingleWakeWord    │  ← single 模式：mel 提取 + ort 推理
              │   │   └─ MelExtractor  │      (mel.rs: FFT + filterbank)
              │   ├─ CompositeEngine   │
              │   └─ NoopEngine        │
              └───────────┬───────────┘
                          │
                    [f32; 3] 置信度 → 阈值比对 → WakeEvent
```

### 5.2 新架构

```
ContinuousListener → RingBuffer (f32 PCM)
                          │
                    wake word thread (每 80ms 读 1280 采样)
                          │
              ┌───────────┴───────────┐
              │   SherpaKws            │  ← 单文件封装
              │   ├─ KeywordSpotter    │      sherpa-onnx 内建特征提取
              │   ├─ Stream            │      流式状态管理
              │   └─ keywords map      │      keyword → WakeWord 映射
              └───────────┬───────────┘
                          │
                    KeywordResult → WakeEvent（含 start_time）
```

### 5.3 线程模型

当前唤醒词线程轮询 RingBuffer → 每 80ms 取一帧推理。改为 sherpa-onnx 后流程：

```
loop {
    // 1. 等待 RingBuffer 有足够数据（≥ 80ms）
    // 2. 读取 PCM 帧
    // 3. stream.accept_waveform(16000, &frame)   ← 替换 engine.process_frame(&frame)
    // 4. while kws.is_ready(&stream) {
    //        kws.decode(&stream)
    //    }
    // 5. if let Some(result) = kws.get_result(&stream) {
    //        if !result.keyword.is_empty() {
    //            send WakeEvent { word, position }
    //        }
    //    }
}
```

sherpa-onnx 的 `decode()` 是增量解码，内部分帧、特征提取、神经网络推理一步完成。`is_ready()` 在积累足够帧后返回 true。`get_result()` 返回完整结果后流状态不变（需显式 `reset()` 清除）。

**关键差异**：sherpa-onnx 的 `get_result()` 在检测到唤醒词后不会自动重置——同一个 stream 会持续累积音频，后续 `decode()` 可能再检测到同一唤醒词。需要我们自己管理 reset 时机（检测到后立即 reset stream）。

## 六、配置设计

### 6.1 简化后的配置

```toml
# ~/.drop-typing.toml

[wakeword]
enabled = true
model_dir = "sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20"  # 内置模型目录名
silence_timeout_ms = 1500       # 唤醒后静音判定录音结束
pre_roll_ms = 500               # 唤醒词前保留的音频时长
ring_buffer_duration_ms = 3000  # 环形缓冲区时长
keywords_threshold = 0.25       # sherpa-onnx 默认检测阈值（全局）
keywords_score = 1.0            # 唤醒词 score boost

# 自定义唤醒词列表（必需，决定哪些关键词触发哪个通道）
[[wakeword.keywords]]
keyword = "DT 打"               # 自然语言关键词
action = "input"                # → 录入通道

[[wakeword.keywords]]
keyword = "DT 修"
action = "repair"               # → 修复通道

[[wakeword.keywords]]
keyword = "DT 按"
action = "command"              # → 指令通道
```

### 6.2 设计要点

- **不再区分 multi/single 模式** —— sherpa-onnx 原生支持一个模型 + 多个 keywords，统一为一个模式
- **keyword → action 映射** 在配置中显式声明，替代之前按通道索引隐式映射的方式
- **keywords.txt 自动生成** —— 程序启动时根据 `[[wakeword.keywords]]` 列表调用 sherpa-onnx 的 tokenizer API 生成 token 格式，或预先提供转换好的文件
- **多模型支持**（可选扩展）：如需同时加载中英文两个模型，可改为 `[[wakeword.models]]` 列表，每个模型配一组 keywords

### 6.3 配置类型（Rust 侧）

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WakewordConfig {
    pub enabled: bool,
    #[serde(default = "default_model_dir")]
    pub model_dir: String,                                // 模型目录名
    #[serde(default = "default_silence_timeout")]
    pub silence_timeout_ms: u64,
    #[serde(default = "default_pre_roll")]
    pub pre_roll_ms: u64,
    #[serde(default = "default_ring_buffer_duration")]
    pub ring_buffer_duration_ms: u64,
    #[serde(default = "default_keywords_threshold")]
    pub keywords_threshold: f32,
    #[serde(default = "default_keywords_score")]
    pub keywords_score: f32,
    #[serde(default)]
    pub keywords: Vec<KeywordEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeywordEntry {
    pub keyword: String,      // 自然语言关键词，如 "DT 打"
    pub action: String,       // "input" | "repair" | "command"
}
```

## 七、步骤计划

### Step 1：添加 sherpa-onnx 依赖（0.5 天）

**文件**：`src-tauri/Cargo.toml`

```toml
# 新增
sherpa-onnx = "1.13"

# 移除（sherpa-onnx 自带 onnxruntime，不再需要手动依赖 ort）
# ort = "2.0.0-rc.13"     ← 删除
# rustfft = "6"           ← 删除（mel.rs 删除后不再需要）
```

确认 `cargo check` 通过，macOS arm64/x64 预编译库下载正常。

### Step 2：准备预训练模型（0.5 天）

1. 下载中英双语 KWS 模型：
   ```bash
   curl -L -o /tmp/kws-model.tar.bz2 \
     https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20.tar.bz2
   tar xf /tmp/kws-model.tar.bz2 -C src-tauri/models/builtin/
   ```

2. 生成 keywords token 文件（一次性，离线操作）：
   ```bash
   # 安装 sherpa-onnx CLI 工具
   pip install sherpa-onnx

   # 把自然语言关键词转为模型 token 格式
   sherpa-onnx-cli text2token \
     --tokens src-tauri/models/builtin/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20/tokens.txt \
     --text "DT 打" "DT 修" "DT 按" \
     --output src-tauri/models/builtin/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20/keywords.txt
   ```

3. 验证模型可加载：写一个最小示例确认 `KeywordSpotter::create()` 成功。

### Step 3：实现 SherpaKws 封装（1 天）

**新文件**：`src-tauri/src/wakeword/sherpa.rs`

核心实现：

```rust
use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig, KeywordResult};
use std::collections::HashMap;
use std::path::Path;

pub struct SherpaKws {
    spotter: KeywordSpotter,
    /// keyword → WakeWord 映射
    keyword_map: HashMap<String, WakeWord>,
}

impl SherpaKws {
    pub fn load(model_dir: &Path, keywords: &[(String, WakeWord)],
                threshold: f32, score: f32) -> anyhow::Result<Self>;

    /// 处理一帧 PCM（f32, 16kHz, 单声道），返回检测到的唤醒词
    pub fn process_frame(&mut self, stream: &mut SherpaKwsStream,
                         frame: &[f32]) -> Option<WakeWord>;

    /// 重置流状态（唤醒词检测到后调用，避免重复触发）
    pub fn reset(&self, stream: &mut SherpaKwsStream);
}

pub struct SherpaKwsStream {
    inner: sherpa_onnx::Stream,
}
```

要点：
- `keyword_map` 把 sherpa-onnx 返回的 `result.keyword` 字符串映射到 `WakeWord` 枚举
- `process_frame()` 内部调 `accept_waveform()` → `decode()` 循环 → `get_result()`
- `reset()` 调 `spotter.reset(&stream.inner)`
- 加载时对每个 keyword 调 `sherpa_onnx::KeywordSpotter::create_stream_with_keywords()`，或统一用 `keywords.txt` 文件

### Step 4：重写 wakeword/mod.rs（0.5 天）

**文件**：`src-tauri/src/wakeword/mod.rs`

- 保留 `WakeWord` 枚举、`WakeEvent` 类型（它们被 pipeline 广泛引用）
- 删除 `WakeWordEngine` trait、`NoopEngine`、`CompositeEngine`
- 新工厂函数 `create_engine()` 返回 `Option<SherpaKws>`
- 模型路径解析：优先 `resource_dir/models/builtin/{model_dir}/`，其次文件系统路径

### Step 5：适配 audio/listener.rs（0.5 天）

**文件**：`src-tauri/src/audio/listener.rs`

- 重写 `start_wake_word()`：
  - 参数改为 `SherpaKws` + 各通道灵敏度（保留灵敏度配置）
  - 内部创建 `SherpaKwsStream`
  - 轮询逻辑改为 `sherpa.process_frame(&mut stream, &frame)`
  - 检测到后立即 `sherpa.reset(&mut stream)` 避免连续触发
- 删除对 `WakeWordEngine` trait 的引用

### Step 6：适配 pipeline.rs（0.5 天）

**文件**：`src-tauri/src/pipeline.rs`

- 唤醒词检测事件处理逻辑基本不变
- 新增使用 `result.start_time`（sherpa-onnx 提供的时间戳）精确定位 RingBuffer 裁切点：
  ```rust
  // 之前：根据 WakeWord 枚举查固定 duration_ms
  // 现在：用 sherpa-onnx 返回的 start_time 计算精确位置
  let wake_start_sample = (result.start_time * 16000.0) as u64;
  ```

### Step 7：更新配置（0.5 天）

**文件**：`src-tauri/src/config.rs`、`config.example.toml`

- 简化 `WakewordConfig`：移除 `mode`/`multi`/`single`，新增 `model_dir`/`keywords`/`keywords_threshold`/`keywords_score`
- 向后兼容：若检测到旧版 multi/single 格式，打印迁移提示
- `config.example.toml` 更新示例

### Step 8：删除旧代码 + 清理依赖（0.5 天）

- 删除 `src-tauri/src/wakeword/mel.rs`
- 删除 `src-tauri/src/wakeword/openwakeword.rs`
- 删除 `src-tauri/models/builtin/alexa_v0.1.onnx`、`hey_jarvis_v0.1.onnx`（旧的 openWakeWord 模型）
- 从 `Cargo.toml` 移除 `ort`、`rustfft`
- 全量 `cargo check` + `cargo build`
- 运行 `npm run tauri dev` 验证编译 + 手动测试唤醒词

### Step 9：验证与测试（0.5 天）

1. **编译验证**：`cargo check` / `cargo build` / `npm run tauri build` 均通过
2. **单元测试**：SherpaKws 加载/推理/重置 基本测试
3. **手动验证**（README 用户验证清单）：
   - 开启 `wakeword.enabled = true`，确认模型加载成功
   - 说唤醒词 → 暂存条弹出 → ASR 转写 → 文本正确
   - 三路唤醒词分别对应三通道（DT 打→录入、DT 修→修复、DT 按→指令）
   - 热键在唤醒词启用时仍正常工作
   - 关闭 `wakeword.enabled` → 行为和现在完全一致（无持续监听）

## 八、风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| 双语模型对 "DT 打/修/按" 检测率不达标 | 唤醒体验差 | 备选：用单语中文模型；或微调模型；降级为纯热键模式 |
| sherpa-onnx 预编译库与 macOS 版本不兼容 | 编译/运行失败 | crate 支持 fallback 源码编译；或锁定已知兼容版本 |
| `ort` 被移除导致其它依赖破坏 | 编译失败 | 全局 `cargo tree` 确认没有其它 crate 依赖 ort |
| 新 crate 首次 build 下载慢 | 首次编译时间长 | 预下载静态库到 CI/本地缓存 |
| keywords token 转换不对齐 | 唤醒词检测不到 | 用法文档已验证 text2token 工具；可写自动化测试对比 token 输出 |
| sherpa-onnx 内存/CPU 开销比 openWakeWord 大 | 资源消耗增加 | 模型大 8 倍但仍只有 3MB；RTF 0.036 仍远低于实时；macOS 上 CPU < 2% |

## 九、后续扩展

sherpa-onnx 带来的额外能力（本次迁移范围内可选、后续迭代再启用）：

1. **内置 VAD**（无需引入 webrtc-vad crate）：
   ```rust
   use sherpa_onnx::VoiceActivityDetector;
   // 可配置 Silero VAD 作为唤醒词前置过滤
   ```

2. **说话人识别**（Phase 3 声纹校验可更简单）：
   sherpa-onnx 提供 Speaker Verification API，比自行集成 WeSpeaker 更统一

3. **更多预训练模型**：官方模型 zoo 持续更新，可无缝升级

4. **CoreML 推理后端**（未来 macOS 专属优化）：
   sherpa-onnx 支持 CoreML delegate，可进一步降低推理延迟和功耗

## 十、时间估算

| 步骤 | 内容 | 时间 |
|------|------|------|
| Step 1 | 添加依赖 | 0.5 天 |
| Step 2 | 准备模型 | 0.5 天 |
| Step 3 | SherpaKws 封装 | 1 天 |
| Step 4 | 重写 mod.rs | 0.5 天 |
| Step 5 | 适配 listener.rs | 0.5 天 |
| Step 6 | 适配 pipeline.rs | 0.5 天 |
| Step 7 | 更新配置 | 0.5 天 |
| Step 8 | 删除旧代码 + 清理 | 0.5 天 |
| Step 9 | 验证与测试 | 0.5 天 |
| **合计** | | **5 天** |

纯新增代码约 320 行，删除约 646 行，净减少约 326 行。

## 十一、与现有文档的关系

- [docs/wake-word-mac.md](docs/wake-word-mac.md) —— 方案设计文档，迁移完成后需更新技术选型章节（openWakeWord → sherpa-onnx）
- [docs/openwakeword-training-guide.md](docs/openwakeword-training-guide.md) —— openWakeWord 训练指南，迁移后可归档或更新为 sherpa-onnx 微调指南
- [CLAUDE.md](CLAUDE.md) —— 需更新模块说明，删除 mel.rs / openwakeword.rs，新增 sherpa.rs

## 十二、不做的事

1. **不引入 VAD** —— 本次迁移范围仅替换唤醒词引擎，VAD 留待后续（sherpa-onnx 已内置 VAD，届时加一个配置开关即可）
2. **不引入说话人识别** —— 声纹校验是 Phase 3，本次不变
3. **不移除 ContinuousListener / RingBuffer** —— 这些是音频基础设施，与唤醒词引擎独立，保留
4. **不改变前端/暂存条** —— UI 零改动
5. **不支持热更新模型** —— 模型切换需重启应用（和现状一致）
