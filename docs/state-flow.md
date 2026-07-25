# 录入与修复功能状态流

## 热键事件层

热键监听（`hotkey/macos.rs` via rdev）发出 6 种事件给 pipeline：

| 事件 | 含义 |
|---|---|
| `TriggerDown` / `TriggerUp` | 右 ⌘ 按下/松开 |
| `RepairDown` / `RepairUp` | 右 ⌥ 按下/松开 |
| `OtherKeyDown` | 录音期间有其它键按下（说明右修饰键被当作组合键修饰符了） |
| `Error(...)` | 监听器运行时错误 |

---

## 配置项

长按/短按/双击行为的判定由两个可配置阈值控制：

| 配置项 | 默认值 | 含义 |
|---|---|---|
| `long_press_threshold_ms` | 150 | 按住时长 ≥ 该值视为长按（录音），否则为短按 |
| `double_press_window_ms` | 350 | 第一次短按松手后，在该时长内再次短按松手视为双击；超时则确认为单击 |

---

## 一、录入通道（右 ⌘，`RecordMode::Input`）

### 1.1 整体状态机

[pipeline.rs:39-54](../src-tauri/src/pipeline.rs#L39-L54) 定义了**三态**：

```
                              ┌──────────────────────────────┐
                              │         PendingCommit        │
                              │     (等待第二击 or 超时)       │
                              │  超时 → commit → Idle        │
                              └──────┬───────────┬───────────┘
                                 ▲ 第二击        ▲ 第一次短按
                                 │ (TriggerDown) │ (trigger_duration < threshold
                                 │               │  && staging 非空)
                                 │               │
Idle  ──TriggerDown──▶  Recording  ──TriggerUp──▶  分发
  ▲                      │    ▲
  │                      │    │
  └──(松手处理完成)────────┘    └──OtherKeyDown(taint)──┘
```

`Recording` 状态包含六个字段：
- `started: Instant` — 按下时刻，用于松手时判定长短按
- `tainted: bool` — 录音是否已被污染（组合键用法）
- `mode: RecordMode` — Input 还是 Repair
- `pending_since: Option<Instant>` — 若本次录音是从 PendingCommit 触发的，携带第一击松手时间（用于双击判定）
- `session / pending_rx` — 实时 ASR 的 WebSocket 会话句柄

`PendingCommit` 状态包含 `since: Instant`（第一次短按松手时刻）。

### 1.2 按下右 ⌘（`TriggerDown`）—— 三种入口

[pipeline.rs:146-254](../src-tauri/src/pipeline.rs#L146-L254) 根据当前状态分三种情况：

#### 入口 A：Idle → Recording（全新按下）

1. 清除上轮异常态、清空 partial 中间结果、清除 repair-note、显示暂存条窗口
2. 创建 PCM 通道 → 录音器 `recorder.start(pcm_tx)` 开始录
3. 如果是 **Realtime 后端**（默认的 fun-asr-realtime）：
   - 创建中间结果转发线程：WebSocket 过来的 sentence 实时推到暂存条弱化展示（`staging.partial(text)`）
   - 创建音频转发线程：缓冲录音数据，等 WebSocket 建连成功后补发 + 续传
   - **后台线程** `start_session` 发起 WebSocket 连接（不阻塞事件循环）
4. 暂存条显示"波形动画"（`staging.set_recording(true)`）
5. 进入 `State::Recording { pending_since: None, ... }`

#### 入口 B：PendingCommit → Recording（第二击）

与入口 A 相同，但**携带第一击时间**：`pending_since = Some(第一次松手时刻)`。

这个时间戳在松手时用于判定是双击还是单击：
- 若 `since.elapsed() < double_press`（第二次松手仍在窗口内）→ 双击
- 若 `since.elapsed() >= double_press` → 视为新一轮单击，重置 PendingCommit

#### 入口 C：Recording → taint（双修饰键）

如果已在录音中（比如同时按了右 ⌘ 和右 ⌥），直接 taint，continue。

### 1.3 录音期间（轮询）

**超阈值判定**（[pipeline.rs:120-125](../src-tauri/src/pipeline.rs#L120-L125)）：每 50ms 检查一次，一旦按住时长 ≥ `long_press_threshold_ms`（默认 150ms），就显示状态徽章 **"识别中"**（不等松手）。

**实时 ASR 中间结果**：WebSocket 持续返回 sentence → 中间结果线程逐条调 `staging.partial(text)` → 前端以弱化样式展示。

**污染检测**：录音期间任何 `OtherKeyDown`（其它键按下）→ `tainted = true`。这处理了「右 ⌘ + Space」这类组合键场景。

### 1.4 松开右 ⌘（`TriggerUp`）—— 三条分支

[pipeline.rs:256-385](../src-tauri/src/pipeline.rs#L256-L385) 是核心分发逻辑：

```
                      松开右 ⌘
                         │
                  ┌──────┴──────┐
             tainted?        duration < threshold? (默认 150ms)
               │                │
         作废录音           ┌───┴───┐
         隐藏窗口         Yes      No
                          │        │
                       短按      长按
                          │        │
                  r.discard()   停止录音 → ASR
                  按模式分发       │
                          ┌──────┼──────┐
                     Realtime   Batch    无后端
                       │         │        │
                 s.finish()  r.stop()  报错
                 取最终文本  整段WAV
                       │     HTTP转写
                       └───┬──┘
                    按 mode 分发
```

#### 分支 A：tainted → 作废

- `r.discard()` 丢弃录音数据
- 清除状态徽章、隐藏暂存条
- 不送 ASR，不提交

#### 分支 B：短按（< threshold）

`r.discard()` 丢弃录音，然后按 `PendingCommit` 状态和 `mode` 分发：

##### B1：录入通道（`RecordMode::Input`）—— 单击 / 双击 判定

```
                        短按松手 (Input mode)
                              │
                     ┌───────┴───────┐
                暂存条为空?      暂存条有内容
                    │                │
               隐藏窗口      ┌──────┴──────┐
                          有 pending_since? (来自 PendingCommit)
                               │                │
                          ┌───┴───┐           No (首次短按)
                    在双击窗口内?        → PendingCommit { since: now }
                         │         │
                        Yes       No (窗口已过)
                         │         │
                    双击：      单击（新）：
                  staging.take()  PendingCommit
                  清空暂存条      { since: now }
                  (暂存条保持显示)
```

三种意图的具体行为：

| 意图 | 触发条件 | 行为 |
|---|---|---|
| **单击** | 第一次短按 → 进入 PendingCommit → 等待 `double_press_window_ms` 超时 | `commit()`：暂存条全文 → 剪贴板 → Cmd+V → 恢复原剪贴板 → 清空 → 隐藏 |
| **双击** | PendingCommit 状态下再次短按，且 `pending_since.elapsed() < double_press_window_ms` | `staging.take()` 清空暂存条，暂存条保持显示（不提交、不隐藏） |
| **窗口过期重来** | PendingCommit 状态下再次短按，但 `pending_since.elapsed() >= double_press_window_ms` | 视为新一轮单击，重新进入 PendingCommit |

##### B2：修复通道（`RecordMode::Repair`）

短按无动作（PRD 第 5 节），仅清除 repair-note 并隐藏窗口。

#### 分支 C：长按（≥ threshold）

这是真正的「语音录入」路径。

**实时后端（默认）**：

1. 从 `pending_rx` 取回建连结果（`session` 句柄）
2. 建连失败 → 报错
3. 建连成功 → `s.finish()` 拿最终转写全文
4. 文本非空 → 按 mode 分发：Input → `clean_and_append()`，Repair → `repair_and_replace()`
5. 文本为空 → 报错 "ASR 返回空文本"

**批量后端（备选 dashscope-http）**：

1. `r.stop()` → 整段 WAV
2. 异步 `spawn_transcribe` → HTTP 转写
3. 转写完成 → 按 mode 分发

### 1.5 PendingCommit 超时 → 单击提交

[pipeline.rs:128-133](../src-tauri/src/pipeline.rs#L128-L133)：轮询循环每 50ms 检查 PendingCommit 状态，一旦 `since.elapsed() >= double_press_window_ms`，确认单击，执行 `commit()`。

```
commit() 详细流程（pipeline.rs:392-409）：

staging.take() 取出全文
  │
  ├─ 为空 → 隐藏窗口，return
  │
  └─ 非空 → injector.paste_text(text)
              │
              ├─ 成功 → staging.committed() + staging.hide()
              │
              └─ 失败 → staging.set_text(text) 回滚内容
                        staging.error(...) 黄底红字
                        （窗口保持可见，内容不丢）
```

`paste_text` 底层（[inject/macos.rs:53-72](../src-tauri/src/inject/macos.rs#L53-L72)）：

1. 读剪贴板当前内容（保存）
2. 写入暂存条文本 → sleep 60ms 等生效
3. 主线程模拟 Cmd+V → sleep 150ms 等目标 App 取走
4. 恢复原剪贴板内容

### 1.6 clean_and_append 详细流程

[pipeline.rs:455-488](../src-tauri/src/pipeline.rs#L455-L488)：

```
ASR 文本
  │
  ├─ 未配置 LLM → staging.append(text) 直接追加
  │
  └─ 配置了 LLM → staging.set_status("润色中")
                   异步调 cleaner.clean(text, strength)
                     │
                     ├─ 成功且非空 → staging.append(cleaned)
                     ├─ 成功但为空 → staging.append(raw) + error("清洗返回空文本，已直出原文")
                     └─ 失败       → staging.append(raw) + error("清洗失败，已直出原文：{e}")
```

关键降级语义：**清洗失败绝不丢内容**，降级为原文追加 + 黄底红字提示。

---

## 二、修复通道（右 ⌥，`RecordMode::Repair`）

修复通道复用同一条 pipeline 事件循环，仅 `mode` 不同导致行为分叉。

### 2.1 录音期间

与录入通道的区别：实时中间结果走 `staging.set_repair_note(text)` 而非 `staging.partial(text)`。前端渲染为一个**独立元素（特殊背景色）**，与暂存条正文分开显示。

### 2.2 松开右 ⌥ 后的分支

| 场景 | 行为 |
|---|---|
| tainted | 同录入：作废，隐藏窗口 |
| 短按 | **无动作**（PRD 第 5 节约定），清除 repair-note，隐藏窗口 |
| 长按 + ASR 结果 | `repair_and_replace()` |
| 长按 + ASR 空 | 报错 "ASR 返回空文本" |

### 2.3 repair_and_replace 详细流程

[pipeline.rs:496-536](../src-tauri/src/pipeline.rs#L496-L536)：

```
ASR 转写的修正指令（instruction）
  │
  ├─ 暂存条为空 → error("暂存条为空，无法修正")
  │
  ├─ 未配置 LLM → error("未配置 LLM，无法使用语音修正")
  │
  └─ 正常路径：
       1. staging.set_repair_note(instruction)  // 展示修正指令（特殊背景色）
       2. staging.set_status("修复中")
       3. 异步 cleaner.repair(original, instruction)
            │
            ├─ 成功且非空 → staging.replace(corrected)
            │                staging.set_repair_note("")  // 清除修正指令展示
            ├─ 成功但为空 → staging.error("修正返回空文本，已保留原文")
            └─ 失败       → staging.error("修正失败，已保留原文：{e}")
```

**修复不丢原文**：失败时原文不动，仅黄底红字提示。

### 2.4 与录入通道的关键差异总结

| | 录入（右 ⌘） | 修复（右 ⌥） |
|---|---|---|
| 实时中间结果 | `staging.partial()` 弱化展示 | `staging.set_repair_note()` 独立展示 |
| 短按 | 单击/双击判定 → 提交或清空 | 无动作，仅隐藏 |
| 长按结果处理 | `clean_and_append` 追加 | `repair_and_replace` 整体替换 |
| LLM 方法 | `cleaner.clean()` | `cleaner.repair()` |
| 失败降级 | 原文追加 | 原文保留不动 |

---

## 三、暂存条状态与前端事件的对应

[staging.rs](../src-tauri/src/staging.rs) 是整个流程的「视图层」，所有状态变化通过 Tauri 事件推给前端：

| Rust 方法 | 前端事件 | 效果 |
|---|---|---|
| `set_recording(bool)` | `drop-typing://recording` | 波形动画 |
| `set_busy(bool)` | `drop-typing://busy` | 半透明 loading 态 |
| `set_status("识别中"/"润色中"/"修复中"/"")` | `drop-typing://status` | 右侧状态徽章 |
| `partial(text)` | `drop-typing://partial` | 实时识别中间结果（弱化样式） |
| `set_repair_note(text)` | `drop-typing://repair-note` | 修正指令独立展示 |
| `append(segment)` | `drop-typing://staging` | 追加文字到暂存条 |
| `replace(text)` | `drop-typing://staging` | 整体替换暂存条 |
| `take()` | `drop-typing://staging` | 取出并清空暂存条 |
| `error(msg)` | `drop-typing://error` | 黄底红字异常态 |
| `show()` | 窗口物理定位 + `win.show()` | 屏幕底部居中显示 |
| `hide()` | `win.hide()` | 隐藏窗口 |
| `committed()` | `drop-typing://committed` | 提交成功反馈 |

窗口定位固定为**屏幕底部居中**（[staging.rs:79-93](../src-tauri/src/staging.rs#L79-L93)），高度从 60px 起撑到 272px 上限后滚动。前端通过 `drop-typing://resize` 事件把内容测量高度回报给 Rust，Rust 做底边不动向上生长的 resize。

---

## 四、完整时序图

### 4.1 长按录入（正常路径）

```
按下右 ⌘
  │
  ├─ staging.show()          窗口底部居中显示
  ├─ staging.clear_error()   清除上轮异常
  ├─ staging.partial("")     清空上轮中间结果
  ├─ recorder.start()        开始录音
  ├─ 后台线程: WebSocket 建连
  ├─ staging.set_recording(true)  波形动画
  │
  ▼  Recording 状态（pending_since = None）
  │
  ├─ [每50ms轮询] 超 threshold → staging.set_status("识别中")
  ├─ [WebSocket 流] → staging.partial(...) 实时中间结果
  │
  ▼  松开右 ⌘ (duration ≥ threshold)
  │
  ├─ state = Idle
  ├─ staging.set_recording(false)
  ├─ 取回 session → s.finish() → 最终文本
  ├─ staging.set_busy(true)
  │
  ├─ LLM 清洗（若有）
  │   └─ staging.set_status("润色中")
  │
  ├─ staging.append(cleaned_text)  文字追加到暂存条
  ├─ staging.set_busy(false)
  ├─ staging.set_status("")
  │
  ▼  Idle（暂存条保持显示，等用户确认）
```

### 4.2 单击提交流程

```
  Idle（暂存条有内容）
  │
  ▼  短按右 ⌘ (duration < threshold)
  │
  ├─ state = PendingCommit { since: now }
  ├─ staging.set_recording(false)
  │
  ▼  等待 double_press_window_ms（默认 350ms）
  │
  ├─ [超时触发] commit()
  │   ├─ staging.take()          取出全文
  │   ├─ injector.paste_text()   剪贴板 → Cmd+V → 恢复
  │   ├─ staging.committed()     成功反馈
  │   └─ staging.hide()          隐藏窗口
  │
  ▼  Idle
```

### 4.3 双击清空流程

```
  Idle（暂存条有内容）
  │
  ▼  第一次短按右 ⌘ (duration₁ < threshold)
  │
  ├─ state = PendingCommit { since: t₀ }
  │
  ▼  第二次短按右 ⌘ (duration₂ < threshold)
  │   且在 double_press_window_ms 内
  │
  ├─ 进入 Recording（pending_since = Some(t₀)）
  ├─ 松手: duration₂ < threshold, pending_since 有值
  ├─ since.elapsed() < double_press → 双击！
  ├─ staging.take()              清空暂存条
  ├─ staging.set_status("")
  │
  ▼  Idle（暂存条保持显示，但已清空）
```

### 4.4 窗口过期重新单击

```
  PendingCommit { since: t₀ }
  │
  ▼  短按右 ⌘，但 t₀ 距今已超过 double_press_window_ms
  │
  ├─ 进入 Recording（pending_since = Some(t₀)）
  ├─ 松手: duration < threshold
  ├─ since.elapsed() >= double_press → 窗口已过
  ├─ state = PendingCommit { since: now }  ← 重置为新一次单击
  │
  ▼  等待超时 → commit
```
