import { listen, emit } from "@tauri-apps/api/event";

const bar = document.getElementById("bar") as HTMLDivElement;
const finalEl = document.getElementById("final") as HTMLSpanElement;
const partEl = document.getElementById("part") as HTMLSpanElement;

const PLACEHOLDER = "按住右 ⌘ 说话，短按提交";

let currentText = "";
let partialText = "";
let committedTimer: number | undefined;

function resize() {
  // 多行自适应：测量内容高度后通知 Rust 侧调整窗口高度
  requestAnimationFrame(() => {
    emit("napkeys://resize", { height: bar.scrollHeight + 12 });
  });
}

function renderText() {
  bar.classList.remove("error");
  if (currentText.trim().length === 0 && partialText.trim().length === 0) {
    bar.classList.add("placeholder");
    finalEl.textContent = PLACEHOLDER;
    partEl.textContent = "";
  } else {
    bar.classList.remove("placeholder");
    finalEl.textContent = currentText;
    partEl.textContent = partialText;
  }
  resize();
}

// 暂存条文本更新（追加/清空）—— 定稿内容，同时清掉中间结果
listen<{ text: string }>("napkeys://staging", (e) => {
  currentText = e.payload.text;
  partialText = "";
  renderText();
});

// 实时识别的中间结果（句内累积，弱化样式）
listen<{ text: string }>("napkeys://partial", (e) => {
  partialText = e.payload.text;
  bar.classList.remove("error");
  renderText();
});

// 录音状态（驱动波形动画）
listen<{ recording: boolean }>("napkeys://recording", (e) => {
  bar.classList.toggle("recording", e.payload.recording);
  resize();
});

// 转写中状态（呼吸效果）
listen<{ busy: boolean }>("napkeys://busy", (e) => {
  bar.classList.toggle("busy", e.payload.busy);
});

// 异常态：黄底红字
listen<{ message: string }>("napkeys://error", (e) => {
  bar.classList.remove("placeholder", "busy", "recording");
  bar.classList.add("error");
  finalEl.textContent = e.payload.message;
  partEl.textContent = "";
  resize();
});

// 提交成功反馈（绿色闪烁）
listen("napkeys://committed", () => {
  window.clearTimeout(committedTimer);
  bar.classList.add("committed");
  committedTimer = window.setTimeout(() => {
    bar.classList.remove("committed");
  }, 350);
});

renderText();
// 通知 Rust 侧前端已就绪，重发启动期间可能错过的状态（如配置/权限错误）
emit("napkeys://ready");
