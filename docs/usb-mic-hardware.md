# USB 麦克风硬件方案

> 状态：方案设计完成，待启动硬件选型

## 目标

制作一款 USB 有线麦克风，集成三个物理按键，通过 USB HID 标准协议与 drop-typing 协同工作。插上即用，无需配对，无需安装驱动。

## 硬件架构

### 设备模型：USB 复合设备（Composite Device）

```
┌─────────────────────────────────────────┐
│              USB Composite Device       │
│                                         │
│  Interface 0: USB Audio Class 1.0      │
│  ├─ Input Terminal: 麦克风              │
│  ├─ Feature Unit: 音量/增益             │
│  └─ Streaming: 16kHz 16bit 单声道 PCM  │
│                                         │
│  Interface 1: USB HID Keyboard         │
│  ├─ IN Endpoint: 按键 Report            │
│  └─ 三个按键 → F13 / F14 / F15         │
│                                         │
│  一条 USB 线缆，两个逻辑功能，互不干扰     │
└─────────────────────────────────────────┘
```

### 数据流

```
麦克风振膜
    ↓ I²S PDM → PCM 转换
ESP32-S3 I2S 外设
    ↓ USB Audio Class 1.0
macOS CoreAudio → cpal → audio/recorder.rs（零改动）

物理按键 (x3)
    ↓ GPIO 中断
ESP32-S3 固件
    ↓ USB HID Keyboard Report (F13/F14/F15)
macOS IOKit → CGEvent → rdev → pipeline.rs（零改动）
```

## 按键定义

三个按键均使用 **HID Keyboard Page (0x07)** 中的高位功能键，不与任何标准键盘冲突：

| 物理按键 | HID Usage ID | 键名 | 功能 |
|---------|-------------|------|------|
| 按键 1 | 0x68 | **F13** | 录音（长按说话、松手上屏） |
| 按键 2 | 0x69 | **F14** | 提交（暂存条 → 粘贴 → 清空） |
| 按键 3 | 0x6A | **F15** | 取消（作废当前录音 / 清空暂存条） |

### 为什么选 F13–F24 区域

- HID 规范定义了 F13–F24（Usage ID 0x68–0x73），但市面上不存在带这些键的物理键盘
- 不会与任何键盘按键冲突，系统不会弹出输入法、特殊字符等副作用
- macOS 内核原生支持，CGEvent 中作为标准 `NSEventTypeKeyDown` 事件
- rdev 无需修改即可捕获
- 不是 Consumer Page（0x0C）的媒体键——媒体键产生 `NSEventTypeSystemDefined`，rdev 捕获受限

## MCU 选型：ESP32-S3

### 推荐方案

| 项目 | 选型 |
|------|------|
| 主控 | **ESP32-S3-WROOM-1**（模组）或 ESP32-S3-DevKitC |
| 理由 | 内置 USB OTG（支持 device 模式）、I2S 外设、原生 TinyUSB 支持 |
| USB | USB 2.0 OTG，可直接枚举为 Audio + HID 复合设备 |
| 音频输入 | I2S 接口直连 MEMS 麦克风 |
| 开发框架 | ESP-IDF（官方，TinyUSB 栈已集成）或 Arduino（更简单，快速原型） |

### 为什么不选其他 MCU

| MCU | 不选的理由 |
|-----|-----------|
| RP2040（Raspberry Pi Pico） | 无原生 I2S，PIO 模拟不稳定；USB Audio 示例少 |
| STM32F4 | 有 USB Audio 例程，但开发生态不如 ESP32 方便，I2S 配置复杂 |
| nRF52840 | USB Audio 支持弱，强项在 BLE，USB 版不需要 BLE |
| ATmega32U4（Arduino Micro） | 无 I2S，无原生 USB Audio Class 支持，32KB flash 放不下音频栈 |

### ESP32-S3 USB OTG 引脚

ESP32-S3 在 USB Device 模式下：
- **GPIO 19**：USB D-（内部有 1.5kΩ 上拉，用于 device 模式）
- **GPIO 20**：USB D+
- 直接接到 USB-C 接口的 D- / D+，不需要外部 PHY 芯片
- 5V 供电从 USB-C 的 VBUS 取电，经板上 LDO 转 3.3V

## 麦克风选型

### 推荐：I2S MEMS 麦克风

| 型号 | 特性 | 约价 |
|------|------|------|
| **INMP441** | I2S 输出、24bit、SNR 61dBA、底部收音 | ¥5 |
| **SPH0645LM4H** | I2S 输出、18bit、SNR 65dBA | ¥6 |
| **ICS-43434** | I2S 输出、24bit、SNR 64dBA | ¥8 |

推荐 **INMP441**：性价比高、ESP-IDF 有现成驱动、Arduino 库成熟。

### 连接方式

```
INMP441          ESP32-S3
  VDD   ────────  3.3V
  GND   ────────  GND
  SD    ────────  GPIO 4  (I2S DIN)
  SCK   ────────  GPIO 5  (I2S BCLK)
  WS    ────────  GPIO 6  (I2S LRCLK)
  L/R   ────────  GND     (左声道)
```

INMP441 输出为 I2S 标准格式，配置为 16kHz 16bit 单声道时：
- BCLK = 16k × 16 × 2 = 512kHz
- LRCLK = 16kHz

## 固件设计

### 方案 A：Arduino 快速原型（推荐先用这个验证）

使用 PlatformIO 或 Arduino IDE，依赖库：
- `espressif/arduino-esp32`（内置 TinyUSB）
- ESP32 的 `USB` 库（软串口 HID）→ **不够，需用 TinyUSB**

Arduino 环境下 ESP32-S3 使用 TinyUSB 作为底层 USB 栈。开箱即支持复合设备。

关键代码结构：

```cpp
#include <TinyUSB_MIDI.h>   // ESP32-S3 的 TinyUSB 封装
// ... 音频 + HID 复合设备配置 ...

// USB Audio: I2S 读取 → USB IN 端点
// USB HID:   GPIO 中断 → HID Report
```

### 方案 B：ESP-IDF 原生（生产推荐）

使用 ESP-IDF 的 `usb_device` 组件 + `tusb`（TinyUSB）：
- `tusb_config.h` 中开启 `CFG_TUD_AUDIO` + `CFG_TUD_HID`
- USB descriptor 手写 composite device topology
- I2S driver 用 ESP-IDF 的 `driver/i2s` 模块

### USB Descriptor 结构（关键）

```
Device Descriptor
├─ VID/PID（缺省可用 ESP32 默认，量产需申请）
├─ Configuration Descriptor
│  ├─ Interface Association Descriptor (IAD)
│  │  ├─ Interface 0: Audio Control (AC)
│  │  │  └─ Standard AC Interface
│  │  ├─ Interface 1: Audio Streaming (AS) - OUT（可选，喇叭）
│  │  ├─ Interface 2: Audio Streaming (AS) - IN（麦克风）
│  │  │  └─ Isochronous Endpoint IN（音频数据）
│  │  └─ Interface 3: HID Keyboard
│  │     ├─ HID Report Descriptor（F13/F14/F15）
│  │     └─ Interrupt Endpoint IN（按键状态）
```

**HID Report Descriptor** 示例（三个按键）：

```c
// Usage Page: 0x07 (Keyboard)
// Report Size: 1, Report Count: 3
// 只上报 F13/F14/F15 对应的 bit
0x05, 0x07,        // Usage Page (Keyboard)
0x19, 0x68,        // Usage Minimum (F13)
0x29, 0x6A,        // Usage Maximum (F15)
0x15, 0x00,        // Logical Minimum (0)
0x25, 0x01,        // Logical Maximum (1)
0x75, 0x01,        // Report Size (1)
0x95, 0x03,        // Report Count (3)
0x81, 0x02,        // Input (Data, Variable, Absolute) — 3 bits
```

## macOS 侧改动

### pipeline.rs 改动（预估 ~50 行）

三个新按键映射到现有状态机：

```rust
// 新增按键匹配（伪代码）
match event {
    KeyEvent::Pressed(Key::F13) => {
        // 录音模式：等同右 ⌘ 长按
        // 按住 → 开始录音，松手 → 送 ASR → 追加暂存条
    }
    KeyEvent::Pressed(Key::F14) => {
        // 提交模式：等同右 ⌘ 短按
        // 暂存条 → 剪贴板 → Cmd+V → 恢复 → 清空
    }
    KeyEvent::Pressed(Key::F15) => {
        // 取消：清空暂存条/作废当前录音
    }
}
```

或通过配置文件热键绑定到已有热键语义中。

### 零改动区域

| 模块 | 原因 |
|------|------|
| `audio/recorder.rs` | macOS 自动识别 USB Audio Class 设备，cpal 通过 CoreAudio 自动发现新输入设备 |
| `hotkey/macos.rs` | rdev 对 F13–F15 的处理和普通按键完全相同 |
| `staging.rs` | 不关心输入来源 |
| `inject/` | 粘贴逻辑不变 |
| 前端 `src/` | 暂存条 UI 不变 |
| `config.rs` | 可加 `[hotkey.usb_mic]` 段做按键映射，也可直接写 F13/F14/F15 到 keyboard 段 |

## 开发阶段

### Phase 1：验证 USB 按键（零成本，不焊接）

用标准 USB 键盘验证 pipeline 改动：
1. 找一把能自定义键位的键盘（如有可编程层），把不用的三个键映射到 F13/F14/F15
2. 或直接用 Karabiner-Elements 把三个组合键映射到 F13/F14/F15
3. 改 `pipeline.rs`，加 F13/F14/F15 监听
4. 手动测试：按 F13 → 录音 → F14 → 粘贴 → F15 → 取消

**目标**：确认 F13–F15 方案在 macOS 上的行为完全符合预期，再投入硬件。

### Phase 2：Arduino 原型（¥80，一个晚上）

硬件清单：
| 物料 | 型号 | 约价 | 用途 |
|------|------|------|------|
| 开发板 | ESP32-S3-DevKitC | ¥35 | 主控 |
| 麦克风 | INMP441 模块 | ¥5 | 音频采集 |
| 按键 | 6×6mm 轻触开关 ×3 | ¥1 | 控制按键 |
| USB 线 | USB-C 数据线 | ¥10 | 连接 Mac |
| 面包板 + 杜邦线 | — | ¥10 | 免焊接 |

固件：Arduino + TinyUSB 复合设备示例。只实现 Audio Class 1.0 + HID Keyboard。

**目标**：面包板跑通"按键 → Mac 收到 F13 → drop-typing 响应"的完整链路。音频质量不做严格要求。

### Phase 3：打板和外壳（¥200–500，一个周末）

- 原理图 + PCB 设计（嘉立创 EDA，免费）
- 嘉立创 5 元打板（双层板 10cm×10cm 以内）
- 3D 打印外壳或成品铝壳改造

### Phase 4：固件打磨

- USB Audio 采样率确认（16kHz vs 48kHz——drop-typing 内部用 16kHz，但 macOS 期望 48kHz 的 USB Audio）
- 增益调试
- 按键去抖优化
- LED 状态指示（录音中 / 暂存条有内容）

## BOM 估算（量产）

| 物料 | 用途 | 单价（百片量） |
|------|------|---------------|
| ESP32-S3-WROOM-1 模组 | 主控 | ¥12 |
| INMP441 MEMS 麦克风 | 音频 | ¥4 |
| USB-C 母座 16P | 连接 | ¥1.5 |
| 轻触开关 ×3 | 按键 | ¥0.3 |
| LED ×2 | 状态指示 | ¥0.2 |
| PCB 双层板 | — | ¥3 |
| 外壳 | — | ¥10 |
| 线缆/螺丝/杂项 | — | ¥5 |
| **合计** | | **¥36** |

## 与 BLE 无线方案的关系

USB 版和未来 BLE 版的 macOS 侧改动几乎完全相同——都是 F13/F14/F15 按键映射。两者的关系：

```
USB 版（当前计划）：
  有线、无配对、USB 供电、快速原型验证

         ↓ 固件和 macOS 代码积累 → 验证产品形态

BLE 版（后续）：
  无线、需配对、电池供电、可加入 DSP 低功耗唤醒词

         ↓ 两颗芯片协同

唤醒词版（终极）：
  DSP（持续监听）→ 唤醒 ESP32 → 推音频流
```

先 USB 后 BLE 的路径不会浪费代码——MCU 侧的 USB Audio + HID 复合设备的 TinyUSB 配置，与 BLE 版的 Audio + HID over GATT 共享同一个上层逻辑。

## 注意事项

1. **16kHz vs 48kHz**：USB Audio Class 规范中，macOS 期望麦克风提供 48kHz 采样率。drop-typing 内部录音用 16kHz。需在 ESP32 侧做重采样（48k→16k），或在 macOS 侧用 AudioUnit 做转换（AVAudioConverter 自动处理，cpal 通常也支持格式转换）。最简单的做法是 ESP32 直接输出 16kHz——CoreAudio 的回采率转换是免费的。

2. **USB 电流**：USB 2.0 标准 500mA。ESP32-S3 + INMP441 峰值 < 200mA，USB-C 口完全够用，不会触发过流保护。

3. **设备名**：USB Product String 设为 "Drop Mic"，macOS 系统报告和声音偏好设置中会显示此名。

4. **VID/PID**：原型阶段可用 ESP32 默认 VID/PID (0x303A)。量产需向 USB-IF 申请（$5,000）或用芯片厂商的子授权——ESP32 模组的 VID 可用于衍生品。如果只是自己用，默认 VID/PID 足够。

5. **macOS 麦克风权限**：USB 音频设备和内置麦克风一样需要 TCC 权限。首次使用时 macOS 会弹窗申请，用户点"好"即可。这和现在 drop-typing 的权限逻辑一致。
