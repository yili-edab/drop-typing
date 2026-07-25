import { listen, emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

const bar = document.getElementById("bar") as HTMLDivElement;
const finalEl = document.getElementById("final") as HTMLSpanElement;
const partEl = document.getElementById("part") as HTMLSpanElement;
const statusEl = document.getElementById("status") as HTMLDivElement;
const repairNoteEl = document.getElementById("repair-note") as HTMLDivElement;
const commandEl = document.getElementById("command") as HTMLDivElement;
const countdownEl = document.getElementById("countdown") as HTMLDivElement;

const PLACEHOLDER = "按住右 ⌘ 说话，短按提交";
const BAR_MAX_HEIGHT = 260;

let currentText = "";
let partialText = "";
let committedTimer: number | undefined;

function resize() {
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      const h = Math.min(bar.scrollHeight, BAR_MAX_HEIGHT) + 12;
      emit("drop-typing://resize", { height: h });
    });
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

// ---- 关闭按钮 ----

document.getElementById("close-btn")!.addEventListener("click", () => {
  bar.classList.remove("busy", "recording");
  emit("drop-typing://close");
  getCurrentWindow().hide();
});

// ---- Rust → 前端事件 ----

// 暂存条文本更新（追加/清空）
listen<{ text: string }>("drop-typing://staging", (e) => {
  currentText = e.payload.text;
  partialText = "";
  renderText();
});

// 实时识别的中间结果（句内累积）
listen<{ text: string }>("drop-typing://partial", (e) => {
  partialText = e.payload.text;
  bar.classList.remove("error");
  renderText();
});

// 录音状态
listen<{ recording: boolean }>("drop-typing://recording", (e) => {
  bar.classList.toggle("recording", e.payload.recording);
});

// 处理中状态（驱动麦克风光晕）
listen<{ busy: boolean }>("drop-typing://busy", (e) => {
  bar.classList.toggle("busy", e.payload.busy);
  resize();
});

// 状态徽章（倾听中 / 识别中 / 润色中 / 修复中）
listen<{ status: string }>("drop-typing://status", (e) => {
  statusEl.textContent = e.payload.status;
  statusEl.classList.toggle("visible", e.payload.status.length > 0);
  resize();
});

// 修复意见展示（M2 修正通道，独立于正文的特殊区块）
listen<{ text: string }>("drop-typing://repair-note", (e) => {
  const text = e.payload.text;
  repairNoteEl.textContent = text ? `修复意见：${text}` : "";
  repairNoteEl.classList.toggle("visible", text.length > 0);
  resize();
});

// 按键指令展示（M4 指令通道）：大字 + 右侧秒级倒计时
listen<{ text: string; seconds: number }>("drop-typing://command", (e) => {
  currentText = "";
  partialText = "";
  bar.classList.remove("placeholder", "error");
  bar.classList.add("command-mode");
  commandEl.textContent = e.payload.text;
  countdownEl.textContent = e.payload.seconds > 0 ? String(e.payload.seconds) : "";
  countdownEl.classList.toggle("visible", e.payload.seconds > 0);
  resize();
});

// 指令倒计时每秒更新
listen<{ seconds: number }>("drop-typing://command-tick", (e) => {
  countdownEl.textContent = e.payload.seconds > 0 ? String(e.payload.seconds) : "";
  countdownEl.classList.toggle("visible", e.payload.seconds > 0);
});

// 清除指令展示（执行完毕 / 新录音开始 / 关闭按钮）
listen("drop-typing://command-clear", () => {
  bar.classList.remove("command-mode");
  commandEl.textContent = "";
  countdownEl.textContent = "";
  countdownEl.classList.remove("visible");
  resize();
});

// 异常态
listen<{ message: string }>("drop-typing://error", (e) => {
  bar.classList.remove("placeholder", "busy", "recording");
  bar.classList.add("error");
  finalEl.textContent = e.payload.message;
  partEl.textContent = "";
  resize();
});

// 提交成功反馈
listen("drop-typing://committed", () => {
  window.clearTimeout(committedTimer);
  bar.classList.add("committed");
  committedTimer = window.setTimeout(() => {
    bar.classList.remove("committed");
  }, 350);
});

renderText();
// 锁定最小宽度：占位文字宽度作为下限，后续文字再少也不会缩窄
requestAnimationFrame(() => {
  bar.style.minWidth = `${bar.getBoundingClientRect().width}px`;
});
emit("drop-typing://ready");
