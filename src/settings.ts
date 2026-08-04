// drop-typing — 设置页面

// Shoelace 暗色模式：检测系统主题
const mq = window.matchMedia('(prefers-color-scheme: dark)');
if (mq.matches) document.documentElement.classList.add('sl-theme-dark');
mq.addEventListener('change', (e) => {
  document.documentElement.classList.toggle('sl-theme-dark', e.matches);
});

import '@shoelace-style/shoelace/dist/components/dialog/dialog.js';
import '@shoelace-style/shoelace/dist/components/alert/alert.js';
import '@shoelace-style/shoelace/dist/components/spinner/spinner.js';
import '@shoelace-style/shoelace/dist/components/button/button.js';
import '@shoelace-style/shoelace/dist/components/input/input.js';
import '@shoelace-style/shoelace/dist/components/icon/icon.js';
import '@shoelace-style/shoelace/dist/components/select/select.js';
import '@shoelace-style/shoelace/dist/components/option/option.js';
import '@shoelace-style/shoelace/dist/components/details/details.js';
import { emit, listen } from '@tauri-apps/api/event';

// ---- DOM ----
const dlgConfirm = document.getElementById('dlg-confirm') as any;
const dlgConfirmMsg = document.getElementById('dlg-confirm-msg')!;
const alertSuccess = document.getElementById('alert-success') as any;
const alertError = document.getElementById('alert-error') as any;

const dlgIntent = document.getElementById('dlg-intent') as any;
const dlgIntentInput = document.getElementById('dlg-intent-input') as any;

const dlgAddStyle = document.getElementById('dlg-add-style') as any;
const dlgAddStyleInput = document.getElementById('dlg-add-style-input') as any;

const baseWrap = document.getElementById('base-editor-wrap')!;
const baseTextarea = document.getElementById('prompt-base') as HTMLTextAreaElement;
const btnBaseReset = document.getElementById('btn-base-reset') as any;
const btnBaseSave = document.getElementById('btn-base-save') as any;
const btnBaseAi = document.getElementById('btn-base-ai') as any;

const styleTabsContainer = document.getElementById('style-tabs')!;
const btnAddStyle = document.getElementById('btn-add-style') as any;
const styleWrap = document.getElementById('style-editor-wrap')!;
const styleTextarea = document.getElementById('style-textarea') as HTMLTextAreaElement;
const btnStyleReset = document.getElementById('btn-style-reset') as any;
const btnStyleSave = document.getElementById('btn-style-save') as any;
const btnStyleAi = document.getElementById('btn-style-ai') as any;

const styleSelectSettings = document.getElementById('style-select-settings') as any;

// ---- 语音控制面板 DOM ----
const cmdCountdown = document.getElementById('cmd-countdown') as any;
const cmdCountdownEffective = document.getElementById('cmd-countdown-effective')!;
const btnCommandSave = document.getElementById('btn-command-save') as any;
const btnCommandReset = document.getElementById('btn-command-reset') as any;

// ---- 高级面板 DOM ----
const asrProvider = document.getElementById('asr-provider') as any;
const asrProtocol = document.getElementById('asr-protocol') as any;
const asrModel = document.getElementById('asr-model') as any;
const asrBaseUrl = document.getElementById('asr-base-url') as any;
const asrApiKey = document.getElementById('asr-api-key') as any;
const llmProvider = document.getElementById('llm-provider') as any;
const llmProtocol = document.getElementById('llm-protocol') as any;
const llmModel = document.getElementById('llm-model') as any;
const llmBaseUrl = document.getElementById('llm-base-url') as any;
const llmApiKey = document.getElementById('llm-api-key') as any;
const llmStrength = document.getElementById('llm-strength') as any;
const millisLongPress = document.getElementById('millis-long-press') as any;
const millisDoublePress = document.getElementById('millis-double-press') as any;
const millisCommandCountdown = document.getElementById('millis-command-countdown') as any;
const btnGeneralSave = document.getElementById('btn-general-save') as any;
const btnMillisSave = document.getElementById('btn-millis-save') as any;
const btnTestAsr = document.getElementById('btn-test-asr') as any;
const btnTestLlm = document.getElementById('btn-test-llm') as any;
const configFileText = document.getElementById('config-file-text') as HTMLTextAreaElement;
const btnConfigReload = document.getElementById('btn-config-reload') as any;
const btnConfigSave = document.getElementById('btn-config-save') as any;

// ---- 唤醒词高级参数 DOM ----
const wwModelDir = document.getElementById('ww-model-dir') as any;
const wwThreshold = document.getElementById('ww-threshold') as any;
const wwScore = document.getElementById('ww-score') as any;
const wwSilence = document.getElementById('ww-silence') as any;
const wwPreRoll = document.getElementById('ww-pre-roll') as any;
const wwRing = document.getElementById('ww-ring') as any;

// ---- 状态 ----
const BUILTIN_STYLE_KEYS = ['high_eq', 'low_eq', 'anti_pua', 'pua'];
const BUILTIN_LABELS: Record<string, string> = {
  high_eq: '高情商', low_eq: '低情商', anti_pua: '反 PUA', pua: 'PUA',
};
const MIN_LOADING_MS = 300;

// 从后端样式列表获取
let stylesList: { key: string; label: string; builtin: boolean }[] = [];
let currentStyle = BUILTIN_STYLE_KEYS[0];
let defaultBase = '';
let defaultStyles: Record<string, string> = {};
let userBase: string | null = null;
let userStyles: Record<string, string> = {};
let loadingKey = '';
let configReady = false;

// ---- 工具 ----

function toast(type: 'success' | 'danger', msg: string) {
  const el = type === 'success' ? alertSuccess : alertError;
  el.innerHTML = msg;
  el.toast();
}

let loadingTimer = 0;
function startLoading(key: string) {
  loadingKey = key; loadingTimer = Date.now();
  baseWrap.classList.toggle('loading', key === 'base');
  styleWrap.classList.toggle('loading', key !== 'base');
  setButtonsDisabled(key, true);
}
function stopLoading() {
  const remain = Math.max(0, MIN_LOADING_MS - (Date.now() - loadingTimer));
  setTimeout(() => {
    baseWrap.classList.remove('loading'); styleWrap.classList.remove('loading');
    setButtonsDisabled('base', false); setButtonsDisabled(currentStyle, false);
    btnBaseSave.textContent = '保存'; btnBaseAi.textContent = 'AI 优化';
    btnStyleSave.textContent = '保存'; btnStyleAi.textContent = 'AI 优化';
    loadingKey = '';
  }, remain);
}
function setButtonsDisabled(key: string, d: boolean) {
  const btns = key === 'base' ? [btnBaseReset, btnBaseSave, btnBaseAi] : [btnStyleReset, btnStyleSave, btnStyleAi];
  btns.forEach(b => { b.disabled = d; });
}

let confirmResolve: (() => void) | null = null;
function showConfirm(msg: string): Promise<boolean> {
  dlgConfirmMsg.textContent = msg;
  return new Promise(resolve => {
    confirmResolve = () => resolve(true);
    const onCancel = () => { resolve(false); dlgConfirm.removeEventListener('sl-after-hide', onCancel); };
    dlgConfirm.addEventListener('sl-after-hide', onCancel, { once: true });
    dlgConfirm.show();
  });
}
document.getElementById('dlg-confirm-yes')!.addEventListener('click', () => { dlgConfirm.hide(); confirmResolve?.(); });
document.getElementById('dlg-confirm-no')!.addEventListener('click', () => dlgConfirm.hide());

function showIntent(placeholder: string): Promise<string | null> {
  dlgIntentInput.placeholder = placeholder;
  dlgIntentInput.value = '';
  return new Promise(resolve => {
    const onOk = () => { dlgIntent.hide(); resolve(dlgIntentInput.value?.trim() || null); };
    const onCancel = () => { resolve(null); dlgIntent.removeEventListener('sl-after-hide', onCancel); };
    dlgIntentInput.addEventListener('sl-input', (e: any) => { dlgIntentInput.value = e.target.value; });
    dlgIntent.addEventListener('sl-after-hide', onCancel, { once: true });
    document.getElementById('dlg-intent-ok')!.addEventListener('click', onOk, { once: true });
    document.getElementById('dlg-intent-cancel')!.addEventListener('click', () => dlgIntent.hide(), { once: true });
    dlgIntent.show();
  });
}

function fillTextareas() {
  baseTextarea.value = userBase ?? defaultBase;
  styleTextarea.value = userStyles[currentStyle] ?? defaultStyles[currentStyle] ?? '';
}

function buildSavePayload(): object {
  const out: Record<string, unknown> = {};
  if (userBase !== null) out.base = userBase;

  const s: Record<string, unknown> = {};
  let has = false;
  for (const k of BUILTIN_STYLE_KEYS) {
    if (userStyles[k] !== undefined) { s[k] = userStyles[k]; has = true; }
  }
  for (const k of Object.keys(userStyles)) {
    if (!BUILTIN_STYLE_KEYS.includes(k)) { s[k] = userStyles[k]; has = true; }
  }
  if (has) out.styles = s;
  return out;
}

function guardReady(): boolean {
  if (!configReady) { toast('danger', '配置尚未加载完成，请稍后再试'); return false; }
  return true;
}

// ---- 样式标签页动态渲染 ----

function renderStyleTabs() {
  styleTabsContainer.innerHTML = '';
  for (const s of stylesList) {
    const btn = document.createElement('sl-button') as any;
    btn.size = 'small';
    btn.variant = s.key === currentStyle ? 'primary' : 'default';
    btn.dataset.style = s.key;

    if (!s.builtin) {
      // 自定义样式：名称 + 删除图标
      const wrapper = document.createElement('span');
      wrapper.className = 'style-tab-custom';
      const nameSpan = document.createElement('span');
      nameSpan.textContent = s.label;
      const del = document.createElement('span');
      del.className = 'style-tab-delete';
      del.textContent = '×';
      del.title = '删除此样式';
      del.addEventListener('click', (e) => {
        e.stopPropagation();
        onDeleteStyle(s.key, s.label);
      });
      wrapper.appendChild(nameSpan);
      wrapper.appendChild(del);
      btn.appendChild(wrapper);
    } else {
      btn.textContent = s.label;
    }

    btn.addEventListener('click', () => {
      selectStyleTab(s.key);
    });

    styleTabsContainer.appendChild(btn);
  }
}

function selectStyleTab(key: string) {
  currentStyle = key;
  const def = stylesList.find(s => s.key === key);
  const isBuiltin = def?.builtin ?? true;

  styleTextarea.value = userStyles[key] ?? defaultStyles[key] ?? '';
  btnStyleReset.textContent = isBuiltin ? '重置' : '删除';
  btnStyleReset.variant = isBuiltin ? 'default' : 'danger';

  renderStyleTabs();
}

async function onDeleteStyle(key: string, label: string) {
  const ok = await showConfirm(`确认删除自定义样式「${label}」？此操作不可恢复。`);
  if (!ok) return;
  if (!guardReady()) return;
  startLoading(key);
  emit('drop-typing://delete-style', { key });
}

function requestStylesList() {
  emit('drop-typing://get-styles');
}

// ---- 面板切换 ----
document.querySelectorAll<HTMLElement>('#menu li[data-panel]').forEach(li => {
  li.addEventListener('click', () => {
    const id = li.dataset.panel!;
    document.querySelectorAll('#menu li[data-panel]').forEach(m => m.classList.remove('active'));
    li.classList.add('active');
    document.querySelectorAll('.panel').forEach(p => p.classList.remove('active'));
    document.getElementById(`panel-${id}`)!.classList.add('active');
    // 懒加载快捷键配置
    if (id === 'shortcut' && !shortcutState.keyboard.trigger) {
      requestShortcutConfig();
    }
    // 懒加载语音控制 / 高级面板配置
    if (id === 'voice-command' && !commandLoaded) {
      requestCommandConfig();
    }
    if (id === 'advanced' && !generalLoaded) {
      requestGeneralConfig();
      requestConfigFile();
    }
  });
});

// ---- 基础按钮 ----
btnBaseReset.addEventListener('click', async () => {
  if (!guardReady()) return;
  const ok = await showConfirm('确认将基础润色提示词重置为系统默认？当前修改将丢失。');
  if (!ok) return;
  startLoading('base');
  emit('drop-typing://reset-prompt', { key: 'base' });
});
btnBaseSave.addEventListener('click', () => {
  if (!guardReady()) return;
  userBase = baseTextarea.value;
  startLoading('base');
  btnBaseSave.textContent = '保存中…';
  emit('drop-typing://save-config', { prompts: buildSavePayload() });
});
btnBaseAi.addEventListener('click', async () => {
  if (!guardReady()) return;
  const text = baseTextarea.value.trim();
  if (!text) { toast('danger', '提示词为空'); return; }
  const intent = await showIntent('输入优化意图，例如：增强结构化、让规则更清晰');
  if (!intent) return;
  startLoading('base');
  btnBaseAi.textContent = '优化中…';
  emit('drop-typing://ai-optimize', { key: 'base', text, intent });
});

// ---- 高级按钮 ----
btnStyleReset.addEventListener('click', async () => {
  if (!guardReady()) return;
  const def = stylesList.find(s => s.key === currentStyle);
  if (!def) return;

  if (!def.builtin) {
    // 自定义样式：删除确认
    const ok = await showConfirm(`确认删除自定义样式「${def.label}」？此操作不可恢复。`);
    if (!ok) return;
    startLoading(currentStyle);
    emit('drop-typing://delete-style', { key: currentStyle });
  } else {
    // 内置样式：重置到默认
    const ok = await showConfirm(`确认将「${def.label}」提示词重置为系统默认？当前修改将丢失。`);
    if (!ok) return;
    startLoading(currentStyle);
    emit('drop-typing://reset-prompt', { key: currentStyle });
  }
});
btnStyleSave.addEventListener('click', () => {
  if (!guardReady()) return;
  userStyles[currentStyle] = styleTextarea.value;
  startLoading(currentStyle);
  btnStyleSave.textContent = '保存中…';
  emit('drop-typing://save-config', { prompts: buildSavePayload() });
});
btnStyleAi.addEventListener('click', async () => {
  if (!guardReady()) return;
  const text = styleTextarea.value.trim();
  if (!text) { toast('danger', '提示词为空'); return; }
  const intent = await showIntent('输入优化意图，例如：让它更真诚、减少说教感');
  if (!intent) return;
  startLoading(currentStyle);
  btnStyleAi.textContent = '优化中…';
  emit('drop-typing://ai-optimize', { key: currentStyle, text, intent });
});

// ---- 添加样式按钮 ----
btnAddStyle.addEventListener('click', () => {
  showAddStyleDialog();
});

async function showAddStyleDialog() {
  dlgAddStyleInput.value = '';
  return new Promise<void>(resolve => {
    const onOk = async () => {
      const name = dlgAddStyleInput.value?.trim() || '';
      if (!name) {
        toast('danger', '样式名称不能为空');
        return;
      }
      // 检查是否与已有名称重复
      if (stylesList.some(s => s.key === name)) {
        toast('danger', '样式名称已存在');
        return;
      }
      if (BUILTIN_STYLE_KEYS.includes(name)) {
        toast('danger', '不能使用与内置样式相同的名称');
        return;
      }
      dlgAddStyle.hide();
      resolve();
      emit('drop-typing://add-style', { key: name });
    };
    const onHide = () => { resolve(); };
    document.getElementById('dlg-add-style-ok')!.addEventListener('click', onOk, { once: true });
    document.getElementById('dlg-add-style-cancel')!.addEventListener('click', () => { dlgAddStyle.hide(); }, { once: true });
    dlgAddStyle.addEventListener('sl-after-hide', onHide, { once: true });
    dlgAddStyle.show();
  });
}

// ---- 事件监听 ----
listen<{
  prompts: { base: string | null; styles: Record<string, string> | null } | null;
  defaults: { base: string | null; styles: Record<string, string> | null } | null;
}>('drop-typing://config', (e) => {
  const d = e.payload.defaults;
  if (d) {
    defaultBase = d.base || '';
    defaultStyles = {};
    if (d.styles) {
      for (const k of BUILTIN_STYLE_KEYS) {
        if (d.styles[k] !== undefined) defaultStyles[k] = d.styles[k];
      }
    }
  }
  const p = e.payload.prompts;
  userBase = p?.base ?? null;
  userStyles = {};
  if (p?.styles) {
    for (const k of Object.keys(p.styles)) {
      userStyles[k] = p.styles[k];
    }
  }
  configReady = true;
  fillTextareas();
  requestStylesList();
});

// 样式列表（后端随时推送）
listen<{
  styles: { key: string; label: string; builtin: boolean }[];
  current: string | null;
}>('drop-typing://styles', (e) => {
  stylesList = e.payload.styles;
  // 确保 currentStyle 仍在列表中
  if (!stylesList.some(s => s.key === currentStyle)) {
    currentStyle = stylesList[0]?.key ?? BUILTIN_STYLE_KEYS[0];
  }
  renderStyleTabs();
  fillTextareas();

  // 设置页「当前润色样式」下拉与暂存条联动
  styleSelectSettings.innerHTML = '<sl-option value="">无</sl-option>';
  for (const s of stylesList) {
    const opt = document.createElement('sl-option');
    opt.value = s.key;
    opt.textContent = s.label;
    styleSelectSettings.appendChild(opt);
  }
  styleSelectSettings.value = e.payload.current || '';
  styleSelectSettings.requestUpdate?.();
});

styleSelectSettings.addEventListener('sl-change', () => {
  const val = styleSelectSettings.value || null;
  emit('drop-typing://select-style', { style: val });
  const label = stylesList.find(s => s.key === val)?.label || '无';
  toast('success', `已切换当前润色样式：${label}`);
});

// 添加样式结果
listen<{ success: boolean; key?: string; error?: string }>(
  'drop-typing://style-added', (e) => {
    if (e.payload.success && e.payload.key) {
      currentStyle = e.payload.key;
      styleTextarea.value = '';
      userStyles[e.payload.key] = '';
      toast('success', `已添加样式「${e.payload.key}」`);
    } else {
      toast('danger', e.payload.error || '添加样式失败');
    }
  }
);

// 删除样式结果
listen<{ success: boolean; key: string; error?: string }>(
  'drop-typing://style-deleted', (e) => {
    stopLoading();
    if (e.payload.success) {
      delete userStyles[e.payload.key];
      if (currentStyle === e.payload.key) {
        currentStyle = stylesList.find(s => s.key !== e.payload.key)?.key ?? BUILTIN_STYLE_KEYS[0];
      }
      fillTextareas();
      // styles 事件会由后端重发以更新 stylesList + 重绘 tabs
      toast('success', `已删除样式「${e.payload.key}」`);
    } else {
      toast('danger', e.payload.error || '删除样式失败');
    }
  }
);

listen<{ success: boolean; error?: string }>('drop-typing://config-saved', (e) => {
  if (e.payload.success) { toast('success', '保存成功'); }
  else { toast('danger', e.payload.error || '保存失败'); }
  stopLoading();
});

listen<{ key: string; default_text: string }>('drop-typing://prompt-reset', (e) => {
  if (e.payload.key === 'base') {
    userBase = null;
    baseTextarea.value = e.payload.default_text;
    toast('success', '已重置为默认值');
  } else {
    delete userStyles[e.payload.key];
    if (currentStyle === e.payload.key) styleTextarea.value = e.payload.default_text;
    toast('success', `「${BUILTIN_LABELS[e.payload.key] || e.payload.key}」已重置`);
  }
  stopLoading();
});

listen<{ key: string; optimized?: string; error?: string }>('drop-typing://ai-optimize-result', (e) => {
  if (e.payload.error) { toast('danger', e.payload.error); }
  else if (e.payload.optimized) {
    if (e.payload.key === 'base') { baseTextarea.value = e.payload.optimized; userBase = e.payload.optimized; }
    else { styleTextarea.value = e.payload.optimized; userStyles[e.payload.key] = e.payload.optimized; }
    toast('success', 'AI 优化完成');
  }
  stopLoading();
});

// ── 语音控制面板（Command） ──────────────────────────────────────────

interface CommandEntry {
  phrase: string;
  modifiers?: string[];
  key?: string;
  modifier?: string;
  name?: string;
  letter?: string;
  script?: string;
  mode?: 'hotkey' | 'script';
}

interface CommandConfigState {
  countdown_ms: number | null;
  action: CommandEntry[];
  modifier: CommandEntry[];
  key: CommandEntry[];
  stop: CommandEntry[];
  homophone: CommandEntry[];
}

const KEY_OPTIONS = [
  ...'ABCDEFGHIJKLMNOPQRSTUVWXYZ',
  ...'0123456789',
  'ENTER', 'SPACE', 'TAB', 'ESC', 'DELETE',
  'UP', 'DOWN', 'LEFT', 'RIGHT',
  ...Array.from({ length: 12 }, (_, i) => `F${i + 1}`),
];
const MOD_OPTIONS = ['Cmd', 'Opt', 'Ctrl', 'Shift'];
const LETTER_OPTIONS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ'.split('');
const CMD_MOD_ORDER = ['Ctrl', 'Opt', 'Shift', 'Cmd'];

function formatComboText(mods: string[], key: string): string {
  const sorted = [...mods]
    .sort((a, b) => CMD_MOD_ORDER.indexOf(a) - CMD_MOD_ORDER.indexOf(b));
  return [...sorted.map(m => m.toUpperCase()), key.toUpperCase()].join(' + ');
}

function formatActionCombo(row: CommandEntry): string {
  const mods = [...(row.modifiers || [])]
    .sort((a, b) => CMD_MOD_ORDER.indexOf(a) - CMD_MOD_ORDER.indexOf(b));
  return formatComboText(mods, row.key || '?');
}

// ── 组合键录制（后端 rdev 全局监听，能捕获被其他软件拦截的组合键） ──

let comboCapturePending = false;
let comboCaptureRow: CommandEntry | null = null;
let comboCaptureTimer: number | null = null;

function startActionRecording(row: CommandEntry) {
  if (comboCapturePending) stopActionRecording();
  comboCapturePending = true;
  comboCaptureRow = row;
  renderLexRows('action');
  emit('drop-typing://start-combo-capture');
  comboCaptureTimer = window.setTimeout(() => {
    if (comboCapturePending) {
      comboCapturePending = false;
      comboCaptureRow = null;
      comboCaptureTimer = null;
      renderLexRows('action');
    }
  }, 11000);
}

function stopActionRecording() {
  if (!comboCapturePending) return;
  emit('drop-typing://stop-combo-capture');
  comboCapturePending = false;
  comboCaptureRow = null;
  if (comboCaptureTimer !== null) {
    clearTimeout(comboCaptureTimer);
    comboCaptureTimer = null;
  }
  renderLexRows('action');
}

listen<{ success: boolean; modifiers: string[]; key: string; error?: string }>(
  'drop-typing://combo-captured', (e) => {
    if (comboCaptureTimer !== null) {
      clearTimeout(comboCaptureTimer);
      comboCaptureTimer = null;
    }
    const row = comboCaptureRow;
    comboCapturePending = false;
    comboCaptureRow = null;
    if (e.payload.success && row) {
      row.modifiers = e.payload.modifiers || [];
      row.key = e.payload.key || row.key || '';
      toast('success', `已捕获：${formatActionCombo(row)}`);
    } else if (e.payload.error && e.payload.error !== '已取消') {
      toast('danger', e.payload.error || '录制失败');
    }
    renderLexRows('action');
  }
);

// ── 可视化键盘选择器（点按组合键，不依赖实体按键） ──

const dlgKeyboard = document.getElementById('dlg-keyboard') as any;
const kbKeysEl = document.getElementById('kb-keys')!;
const kbPreviewText = document.getElementById('kb-preview-text')!;
const kbOk = document.getElementById('kb-ok') as any;
const kbCancel = document.getElementById('kb-cancel') as any;
const kbClear = document.getElementById('kb-clear') as any;

const KB_MOD_KEYS = ['Ctrl', 'Opt', 'Cmd', 'Shift'];
const KB_LAYOUT = [
  ['Esc', 'F1', 'F2', 'F3', 'F4', 'F5', 'F6', 'F7', 'F8', 'F9', 'F10', 'F11', 'F12'],
  ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0', '⌫'],
  ['Tab', 'Q', 'W', 'E', 'R', 'T', 'Y', 'U', 'I', 'O', 'P'],
  ['CapsLock', 'A', 'S', 'D', 'F', 'G', 'H', 'J', 'K', 'L', 'Enter'],
  ['Shift', 'Z', 'X', 'C', 'V', 'B', 'N', 'M', 'Shift'],
  ['Ctrl', 'Opt', 'Cmd', 'Space', 'Cmd', 'Opt', 'Ctrl'],
  ['←', '↓', '↑', '→'],
];

let keyboardRow: CommandEntry | null = null;
let keyboardMods: string[] = [];
let keyboardKey: string | null = null;

function kbKeySupported(label: string): boolean {
  const value = kbKeyValue(label);
  if (/^[A-Z0-9]$/.test(value)) return true;
  if (/^F([1-9]|1[0-2])$/.test(value)) return true;
  return ['ENTER', 'SPACE', 'TAB', 'ESC', 'DELETE', 'UP', 'DOWN', 'LEFT', 'RIGHT'].includes(value);
}

function kbKeyValue(label: string): string {
  if (label === '⌫') return 'DELETE';
  const arrowMap: Record<string, string> = {
    '←': 'LEFT', '↓': 'DOWN', '↑': 'UP', '→': 'RIGHT',
  };
  if (arrowMap[label]) return arrowMap[label];
  if (label === 'Esc') return 'ESC';
  return label.toUpperCase();
}

function renderKeyboard() {
  kbKeysEl.innerHTML = '';
  for (const row of KB_LAYOUT) {
    const rowEl = document.createElement('div');
    rowEl.className = 'kb-row';
    for (const label of row) {
      const isMod = KB_MOD_KEYS.includes(label);
      if (isMod) {
        const btn = document.createElement('button');
        btn.className = 'kb-mod kb-key' + (keyboardMods.includes(label) ? ' active' : '');
        btn.textContent = label;
        btn.addEventListener('click', () => {
          const i = keyboardMods.indexOf(label);
          if (i >= 0) keyboardMods.splice(i, 1);
          else keyboardMods.push(label);
          renderKeyboard();
        });
        rowEl.appendChild(btn);
        continue;
      }

      const supported = kbKeySupported(label);
      const btn = document.createElement('button');
      btn.className = 'kb-key'
        + (keyboardKey === kbKeyValue(label) ? ' active' : '')
        + (label === 'Space' ? ' kb-wide' : '')
        + (supported ? '' : ' kb-disabled');
      btn.textContent = label;
      btn.title = supported ? '' : '暂不支持该按键';
      btn.disabled = !supported;
      btn.addEventListener('click', () => {
        keyboardKey = kbKeyValue(label);
        renderKeyboard();
      });
      rowEl.appendChild(btn);
    }
    kbKeysEl.appendChild(rowEl);
  }

  kbPreviewText.textContent = formatComboText(keyboardMods, keyboardKey || '?');
}

function openKeyboard(row: CommandEntry) {
  keyboardRow = row;
  keyboardMods = [...(row.modifiers || [])];
  keyboardKey = row.key || null;
  renderKeyboard();
  dlgKeyboard.show();
}

kbOk.addEventListener('click', () => {
  if (!keyboardKey) {
    toast('danger', '请先点击一个普通键');
    return;
  }
  if (keyboardRow) {
    keyboardRow.modifiers = [...keyboardMods]
      .sort((a, b) => CMD_MOD_ORDER.indexOf(a) - CMD_MOD_ORDER.indexOf(b));
    keyboardRow.key = keyboardKey;
    toast('success', `已选择：${formatActionCombo(keyboardRow)}`);
  }
  dlgKeyboard.hide();
  renderLexRows('action');
});

kbCancel.addEventListener('click', () => dlgKeyboard.hide());

kbClear.addEventListener('click', () => {
  keyboardMods = [];
  keyboardKey = null;
  renderKeyboard();
});

let commandConfig: CommandConfigState = {
  countdown_ms: null,
  action: [],
  modifier: [],
  key: [],
  stop: [],
  homophone: [],
};
let commandLoaded = false;

function requestCommandConfig() {
  emit('drop-typing://get-command-config');
}

function commandRows(kind: string): CommandEntry[] {
  return (commandConfig as any)[kind] || [];
}

function addLexRow(kind: string) {
  const empty: CommandEntry =
    kind === 'action' ? { phrase: '', modifiers: ['Cmd'], key: 'C' }
    : kind === 'modifier' ? { phrase: '', modifier: 'Cmd' }
    : kind === 'key' ? { phrase: '', name: 'DELETE' }
    : kind === 'homophone' ? { phrase: '', letter: 'A' }
    : { phrase: '' };
  (commandConfig as any)[kind].push(empty);
  renderLexRows(kind);
}

function makeKeySelect(initial: string, onChange: (v: string) => void): any {
  const sel = document.createElement('sl-select');
  sel.className = 'lex-key';
  sel.size = 'small';
  for (const k of KEY_OPTIONS) {
    const o = document.createElement('sl-option');
    o.value = k;
    o.textContent = k;
    sel.appendChild(o);
  }
  sel.value = initial;
  sel.addEventListener('sl-change', (e: any) => onChange(e.target.value || initial));
  return sel;
}

function renderLexRows(kind: string) {
  const wrap = document.querySelector(`.lex-block[data-kind="${kind}"] .lex-rows`)!;
  wrap.innerHTML = '';
  commandRows(kind).forEach((row, idx) => {
    const el = document.createElement('div');
    el.className = 'lex-row';

    const phrase = document.createElement('sl-input');
    phrase.className = 'lex-phrase';
    phrase.size = 'small';
    phrase.placeholder = '短语';
    phrase.value = row.phrase || '';
    phrase.addEventListener('sl-input', (e: any) => {
      row.phrase = e.target.value?.trim() || '';
    });
    el.appendChild(phrase);

    if (kind === 'action') {
      // 执行方式：默认快捷键，预留脚本执行钩子
      const modeSel = document.createElement('sl-select');
      modeSel.className = 'lex-mode';
      modeSel.size = 'small';
      const optHot = document.createElement('sl-option');
      optHot.value = 'hotkey';
      optHot.textContent = '快捷键';
      const optScr = document.createElement('sl-option');
      optScr.value = 'script';
      optScr.textContent = '执行脚本';
      modeSel.appendChild(optHot);
      modeSel.appendChild(optScr);
      const isScript = row.mode === 'script' || !!row.script;
      modeSel.value = isScript ? 'script' : 'hotkey';
      modeSel.addEventListener('sl-change', (e: any) => {
        const v = e.target.value;
        row.mode = v;
        if (v === 'hotkey') row.script = '';
        renderLexRows('action');
      });
      el.appendChild(modeSel);

      if (isScript) {
        const scriptInput = document.createElement('sl-input');
        scriptInput.className = 'lex-script';
        scriptInput.size = 'small';
        scriptInput.placeholder = '/path/to/script.sh';
        scriptInput.value = row.script || '';
        scriptInput.addEventListener('sl-input', (e: any) => {
          row.script = e.target.value?.trim() || '';
        });
        el.appendChild(scriptInput);
      } else {
        // 快捷键：单个只读展示（非输入框）+ 直接录制
        const comboDisplay = document.createElement('span');
        comboDisplay.className = 'lex-combo-display' + (row.key ? '' : ' empty');
        comboDisplay.textContent = row.key ? formatActionCombo(row) : '未设置';
        el.appendChild(comboDisplay);

        const recBtn = document.createElement('sl-button');
        recBtn.className = 'lex-rec';
        recBtn.size = 'small';
        const recordingThis = comboCapturePending && comboCaptureRow === row;
        recBtn.variant = recordingThis ? 'danger' : 'neutral';
        recBtn.textContent = recordingThis ? '取消' : '录制';
        recBtn.addEventListener('click', () => {
          if (comboCapturePending && comboCaptureRow === row) {
            stopActionRecording();
          } else {
            startActionRecording(row);
          }
        });
        el.appendChild(recBtn);

        const kbBtn = document.createElement('sl-button');
        kbBtn.className = 'lex-kb';
        kbBtn.size = 'small';
        kbBtn.variant = 'neutral';
        kbBtn.textContent = '键盘选择';
        kbBtn.addEventListener('click', () => openKeyboard(row));
        el.appendChild(kbBtn);
      }
    } else if (kind === 'modifier') {
      const modSel = document.createElement('sl-select');
      modSel.className = 'lex-key';
      modSel.size = 'small';
      for (const m of MOD_OPTIONS) {
        const o = document.createElement('sl-option');
        o.value = m;
        o.textContent = m;
        modSel.appendChild(o);
      }
      modSel.value = row.modifier || 'Cmd';
      modSel.addEventListener('sl-change', (e: any) => {
        row.modifier = e.target.value || 'Cmd';
      });
      el.appendChild(modSel);
    } else if (kind === 'key') {
      el.appendChild(makeKeySelect(row.name || 'DELETE', v => { row.name = v; }));
    } else if (kind === 'homophone') {
      const letterSel = document.createElement('sl-select');
      letterSel.className = 'lex-key';
      letterSel.size = 'small';
      for (const l of LETTER_OPTIONS) {
        const o = document.createElement('sl-option');
        o.value = l;
        o.textContent = l;
        letterSel.appendChild(o);
      }
      letterSel.value = row.letter || 'A';
      letterSel.addEventListener('sl-change', (e: any) => {
        row.letter = e.target.value || 'A';
      });
      el.appendChild(letterSel);
    }

    const del = document.createElement('sl-button');
    del.className = 'lex-del';
    del.size = 'small';
    del.variant = 'danger';
    del.textContent = '删除';
    del.addEventListener('click', () => {
      (commandConfig as any)[kind].splice(idx, 1);
      renderLexRows(kind);
    });
    el.appendChild(del);

    wrap.appendChild(el);
  });
}

function renderAllLex() {
  for (const kind of ['action', 'modifier', 'key', 'stop', 'homophone']) {
    renderLexRows(kind);
  }
}

document.querySelectorAll<HTMLElement>('.lex-add').forEach(btn => {
  btn.addEventListener('click', () => addLexRow(btn.dataset.kind || ''));
});

function buildCommandPayload() {
  const nonEmpty = <T>(arr: T[]): T[] => arr.filter((x: any) => (x.phrase || '').trim());
  return {
    countdown_ms: cmdCountdown.value ? parseInt(cmdCountdown.value, 10) : null,
    action: nonEmpty(commandConfig.action).map(a => ({
      phrase: a.phrase,
      modifiers: a.modifiers || [],
      key: a.key || '',
      script: a.script || null,
    })),
    modifier: nonEmpty(commandConfig.modifier),
    key: nonEmpty(commandConfig.key),
    stop: nonEmpty(commandConfig.stop),
    homophone: nonEmpty(commandConfig.homophone),
  };
}

btnCommandSave.addEventListener('click', () => {
  btnCommandSave.textContent = '保存中…';
  btnCommandSave.disabled = true;
  emit('drop-typing://save-command-config', buildCommandPayload());
});

btnCommandReset.addEventListener('click', async () => {
  const ok = await showConfirm('确认清空用户指令词表并恢复内置默认？当前修改将丢失。');
  if (!ok) return;
  commandConfig = { countdown_ms: null, action: [], modifier: [], key: [], stop: [], homophone: [] };
  cmdCountdown.value = '';
  renderAllLex();
  toast('success', '已清空，点击「保存」生效');
});

listen<{ config: CommandConfigState; effective_command_countdown_ms: number }>(
  'drop-typing://command-config', (e) => {
    const c = e.payload.config || {};
    commandConfig = {
      countdown_ms: c.countdown_ms ?? null,
      action: (c.action || []).map((a: any) => ({
        ...a,
        script: a.script || '',
        mode: a.script ? 'script' : 'hotkey',
      })),
      modifier: c.modifier || [],
      key: c.key || [],
      stop: c.stop || [],
      homophone: c.homophone || [],
    };
    cmdCountdown.value = commandConfig.countdown_ms ? String(commandConfig.countdown_ms) : '';
    cmdCountdownEffective.textContent = String(e.payload.effective_command_countdown_ms ?? 2000);
    renderAllLex();
    commandLoaded = true;
  }
);

listen<{ success: boolean; error?: string }>('drop-typing://command-config-saved', (e) => {
  btnCommandSave.textContent = '保存';
  btnCommandSave.disabled = false;
  if (e.payload.success) {
    toast('success', '语音控制配置已保存并生效');
  } else {
    toast('danger', e.payload.error || '保存失败');
  }
});

// ── 语音唤醒面板 ─────────────────────────────────────────────────────

// DOM
const wakewordList = document.getElementById('wakeword-list')!;
const btnAddWakeword = document.getElementById('btn-add-wakeword') as any;
const btnWakewordSave = document.getElementById('btn-wakeword-save') as any;
const btnWakewordReset = document.getElementById('btn-wakeword-reset') as any;
const btnWakewordTokens = document.getElementById('btn-wakeword-tokens') as any;
const dlgTokens = document.getElementById('dlg-tokens') as any;
const dlgTokensContent = document.getElementById('dlg-tokens-content')!;
const dlgTokensClose = document.getElementById('dlg-tokens-close') as any;

// 状态
interface WakewordEntry {
  keyword: string;
  action: string;
}
let wakewordEntries: WakewordEntry[] = [];
let wakewordDefaults: WakewordEntry[] = [];
let wakewordHasCustom = false;

const ACTION_OPTIONS = [
  { value: 'input', label: '录入 (Input)' },
  { value: 'repair', label: '修复 (Repair)' },
  { value: 'command', label: '指令 (Command)' },
  { value: 'commit', label: '确认 (Commit)' },
  { value: 'clear', label: '清空 (Clear)' },
];

// 渲染唤醒词列表
function renderWakewordList() {
  wakewordList.innerHTML = '';
  wakewordEntries.forEach((entry, idx) => {
    const row = document.createElement('div');
    row.className = 'wakeword-row';

    // 关键词输入
    const input = document.createElement('sl-input');
    input.className = 'kw-text';
    input.size = 'small';
    input.placeholder = '例如 DT打';
    input.value = entry.keyword;
    input.addEventListener('sl-input', (e: any) => {
      wakewordEntries[idx].keyword = e.target.value?.trim() || '';
    });

    // action 下拉菜单
    const select = document.createElement('sl-select');
    select.className = 'kw-action';
    select.size = 'small';
    select.value = entry.action;
    select.addEventListener('sl-change', (e: any) => {
      wakewordEntries[idx].action = e.target.value;
    });
    ACTION_OPTIONS.forEach(opt => {
      const option = document.createElement('sl-option');
      option.value = opt.value;
      option.textContent = opt.label;
      select.appendChild(option);
    });

    // 删除按钮
    const delBtn = document.createElement('sl-button');
    delBtn.className = 'kw-delete';
    delBtn.size = 'small';
    delBtn.variant = 'danger';
    delBtn.textContent = '删除';
    delBtn.addEventListener('click', () => {
      wakewordEntries.splice(idx, 1);
      renderWakewordList();
    });

    row.appendChild(input);
    row.appendChild(select);
    row.appendChild(delBtn);
    wakewordList.appendChild(row);
  });
}

// 获取当前唤醒词配置
function requestWakewordConfig() {
  emit('drop-typing://get-wakeword-config');
}

// 添加唤醒词
btnAddWakeword.addEventListener('click', () => {
  wakewordEntries.push({ keyword: '', action: 'input' });
  renderWakewordList();
});

// 保存
btnWakewordSave.addEventListener('click', () => {
  btnWakewordSave.textContent = '保存中…';
  btnWakewordSave.disabled = true;
  const keywords = wakewordEntries.filter(e => e.keyword.trim());
  emit('drop-typing://save-wakeword-config', {
    keywords,
    enabled: keywords.length > 0,
    advanced: {
      model_dir: wwModelDir.value,
      keywords_threshold: parseFloat(wwThreshold.value),
      keywords_score: parseFloat(wwScore.value),
      silence_timeout_ms: parseInt(wwSilence.value, 10),
      pre_roll_ms: parseInt(wwPreRoll.value, 10),
      ring_buffer_duration_ms: parseInt(wwRing.value, 10),
    },
  });
});

// 重置
btnWakewordReset.addEventListener('click', async () => {
  const ok = await showConfirm('确认将唤醒词重置为默认值（DT打/DT修/DT控）？当前修改将丢失。');
  if (!ok) return;
  emit('drop-typing://reset-wakeword-config');
});

// 查看 token
btnWakewordTokens.addEventListener('click', () => {
  const keywords = wakewordEntries.filter(e => e.keyword.trim());
  if (keywords.length === 0) {
    toast('danger', '请先添加至少一个唤醒词');
    return;
  }
  btnWakewordTokens.textContent = '计算中…';
  btnWakewordTokens.disabled = true;
  dlgTokensContent.textContent = '计算中…';
  dlgTokens.show();
  emit('drop-typing://preview-wakeword-tokens', { keywords });
});

dlgTokensClose.addEventListener('click', () => dlgTokens.hide());

// ── 重启应用 ───────────────────────────────────────────────────────

const btnRestart = document.getElementById('btn-restart') as any;
btnRestart.addEventListener('click', async () => {
  const ok = await showConfirm('确认重启应用？未保存的修改将丢失。');
  if (!ok) return;
  emit('drop-typing://restart');
});

// ── 事件监听 ───────────────────────────────────────────────────────
listen<any>('drop-typing://wakeword-config', (e) => {
  const d = e.payload;
  wakewordDefaults = d.defaults || [];
  wakewordHasCustom = d.has_custom;
  if (d.keywords && d.keywords.length > 0) {
    wakewordEntries = d.keywords.map((k: any) => ({
      keyword: k.keyword,
      action: k.action,
    }));
  } else {
    wakewordEntries = wakewordDefaults.map((k: any) => ({
      keyword: k.keyword,
      action: k.action,
    }));
  }
  renderWakewordList();

  const adv = d.advanced || {};
  wwModelDir.value = adv.model_dir || '';
  wwThreshold.value = adv.keywords_threshold ?? 0.25;
  wwScore.value = adv.keywords_score ?? 1.0;
  wwSilence.value = adv.silence_timeout_ms ?? 1500;
  wwPreRoll.value = adv.pre_roll_ms ?? 500;
  wwRing.value = adv.ring_buffer_duration_ms ?? 3000;
});

listen<any>('drop-typing://wakeword-saved', (e) => {
  btnWakewordSave.textContent = '保存';
  btnWakewordSave.disabled = false;
  if (e.payload.success) {
    toast('success', '唤醒词配置已保存');
    promptRestart('唤醒词配置需要重启应用后才能生效，是否立即重启？');
  } else {
    toast('danger', e.payload.error || '保存失败');
  }
});

listen<any>('drop-typing://wakeword-reset', (e) => {
  if (e.payload.success) {
    wakewordEntries = wakewordDefaults.map((k: any) => ({
      keyword: k.keyword,
      action: k.action,
    }));
    renderWakewordList();
    toast('success', '已重置为默认值。请重启应用使配置生效。');
  } else {
    toast('danger', e.payload.error || '重置失败');
  }
});

listen<any>('drop-typing://wakeword-tokens', (e) => {
  btnWakewordTokens.textContent = '查看 Token';
  btnWakewordTokens.disabled = false;
  if (e.payload.error) {
    dlgTokensContent.textContent = `错误：${e.payload.error}`;
  } else if (e.payload.lines) {
    dlgTokensContent.textContent = e.payload.lines.join('\n');
  }
});

// 页面加载时请求唤醒词配置
requestWakewordConfig();

// ── 快捷键面板 ─────────────────────────────────────────────────────

// ---- 常量 ----

const SHORTCUT_CHANNELS = [
  { key: 'trigger', label: '录入 (Trigger)', desc: '长按录音，短按提交' },
  { key: 'repair',  label: '修正 (Repair)',  desc: '修正/控制通道' },
  { key: 'command', label: '指令 (Command)', desc: '指令/修复通道' },
  { key: 'cancel',  label: '取消 (Cancel)',  desc: '清空暂存条' },
] as const;

const MOUSE_CHANNELS = [
  { key: 'trigger', label: '录入 (Trigger)', desc: '前进键 (Forward / X2)' },
  { key: 'repair',  label: '修正 (Repair)',  desc: '后退键 (Back / X1)' },
] as const;

// DOM event.code → rdev 键名映射
const DOM_CODE_TO_RDEV: Record<string, string> = {
  // 修饰键
  MetaRight: 'MetaRight', MetaLeft: 'MetaLeft',
  AltRight: 'AltGr', AltLeft: 'Alt',
  ShiftRight: 'ShiftRight', ShiftLeft: 'ShiftLeft',
  ControlRight: 'ControlRight', ControlLeft: 'ControlLeft',
  // 导航
  Escape: 'Escape', Space: 'Space', Tab: 'Tab',
  Enter: 'Return', Backspace: 'Backspace', Delete: 'Delete',
  ArrowUp: 'UpArrow', ArrowDown: 'DownArrow',
  ArrowLeft: 'LeftArrow', ArrowRight: 'RightArrow',
  Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown',
  Insert: 'Insert', CapsLock: 'CapsLock',
  NumLock: 'NumLock', ScrollLock: 'ScrollLock',
  Pause: 'Pause', PrintScreen: 'PrintScreen',
  // 功能键
  F1:'F1',F2:'F2',F3:'F3',F4:'F4',F5:'F5',F6:'F6',
  F7:'F7',F8:'F8',F9:'F9',F10:'F10',F11:'F11',F12:'F12',
  // 字母键
  KeyA:'KeyA',KeyB:'KeyB',KeyC:'KeyC',KeyD:'KeyD',KeyE:'KeyE',
  KeyF:'KeyF',KeyG:'KeyG',KeyH:'KeyH',KeyI:'KeyI',KeyJ:'KeyJ',
  KeyK:'KeyK',KeyL:'KeyL',KeyM:'KeyM',KeyN:'KeyN',KeyO:'KeyO',
  KeyP:'KeyP',KeyQ:'KeyQ',KeyR:'KeyR',KeyS:'KeyS',KeyT:'KeyT',
  KeyU:'KeyU',KeyV:'KeyV',KeyW:'KeyW',KeyX:'KeyX',KeyY:'KeyY',
  KeyZ:'KeyZ',
  // 数字键
  Digit0:'Num0',Digit1:'Num1',Digit2:'Num2',Digit3:'Num3',Digit4:'Num4',
  Digit5:'Num5',Digit6:'Num6',Digit7:'Num7',Digit8:'Num8',Digit9:'Num9',
  // 符号键
  Backquote: 'BackQuote', Minus: 'Minus', Equal: 'Equal',
  BracketLeft: 'LeftBracket', BracketRight: 'RightBracket',
  Semicolon: 'SemiColon', Quote: 'Quote',
  Backslash: 'BackSlash', IntlBackslash: 'IntlBackslash',
  Comma: 'Comma', Period: 'Dot', Slash: 'Slash',
  // 小键盘
  NumpadEnter: 'KpReturn',
  NumpadSubtract: 'KpMinus', NumpadAdd: 'KpPlus',
  NumpadMultiply: 'KpMultiply', NumpadDivide: 'KpDivide',
  NumpadDecimal: 'KpDelete',
  Numpad0:'Kp0',Numpad1:'Kp1',Numpad2:'Kp2',Numpad3:'Kp3',Numpad4:'Kp4',
  Numpad5:'Kp5',Numpad6:'Kp6',Numpad7:'Kp7',Numpad8:'Kp8',Numpad9:'Kp9',
};

// 家族名平台显示名称
const FAMILY_DISPLAY: Record<string, Record<string, string>> = {
  macos:   { Control: 'Ctrl', Meta: 'Cmd', Alt: 'Opt', Shift: 'Shift' },
  windows: { Control: 'Ctrl', Meta: 'Win', Alt: 'Alt',  Shift: 'Shift' },
};

// rdev 精确键 → 显示名称
const KEY_DISPLAY_BY_PLATFORM: Record<string, Record<string, string>> = {
  macos: {
    MetaRight: '右 Cmd', MetaLeft: '左 Cmd',
    AltGr: '右 Opt', Alt: '左 Opt',
    ShiftRight: '右 Shift', ShiftLeft: '左 Shift',
    ControlRight: '右 Ctrl', ControlLeft: '左 Ctrl',
  },
  windows: {
    MetaRight: '右 Win', MetaLeft: '左 Win',
    AltGr: 'AltGr', Alt: 'Alt',
    ShiftRight: '右 Shift', ShiftLeft: '左 Shift',
    ControlRight: '右 Ctrl', ControlLeft: '左 Ctrl',
  },
};
const KEY_DISPLAY_COMMON: Record<string, string> = {
  Escape: 'Esc', Space: '空格', Tab: 'Tab', Return: 'Enter',
  Backspace: 'Backspace', Delete: 'Delete',
  CapsLock: 'CapsLock',
  UpArrow: '↑', DownArrow: '↓', LeftArrow: '←', RightArrow: '→',
  Home: 'Home', End: 'End', PageUp: 'PgUp', PageDown: 'PgDn',
  Insert: 'Insert', NumLock: 'NumLock',
  Pause: 'Pause', PrintScreen: 'PrtSc',
};

function keySpecDisplay(name: string, platform: string): string {
  // 家族名
  if (['Control', 'Meta', 'Alt', 'Shift'].includes(name)) {
    return FAMILY_DISPLAY[platform]?.[name] ?? name;
  }
  // 平台特定精确键
  const pd = KEY_DISPLAY_BY_PLATFORM[platform];
  if (pd?.[name]) return pd[name];
  // 通用精确键
  if (KEY_DISPLAY_COMMON[name]) return KEY_DISPLAY_COMMON[name];
  // KeyA → A, Num1 → 1, F1 → F1
  if (name.startsWith('Key') && name.length === 4) return name[3];
  if (name.startsWith('Num') && name.length === 4) return name[3];
  if (name.startsWith('Kp') && name.length === 3) return 'Num' + name[2];
  return name;
}

function formatShortcut(keys: string[], platform: string): string {
  if (!keys || keys.length === 0) return '未设置';
  return keys.map(k => keySpecDisplay(k, platform)).join(' + ');
}

// ---- 状态 ----

interface ShortcutState {
  platform: 'macos' | 'windows';
  keyboard: Record<string, string[]>;
  mouse: Record<string, string | null>;
  defaults: {
    keyboard: Record<string, string[]>;
    mouse: Record<string, string | null>;
  };
}

let shortcutState: ShortcutState = {
  platform: 'macos',
  keyboard: {},
  mouse: {},
  defaults: { keyboard: {}, mouse: {} },
};

let recordingChannel: string | null = null;
let recordingGroup: 'keyboard' | null = null;
let recordingTeardown: (() => void) | null = null;

// ---- DOM ----

const keyboardChannelsEl = document.getElementById('keyboard-channels')!;
const mouseChannelsEl = document.getElementById('mouse-channels')!;
const platformHintEl = document.getElementById('platform-hint')!;
const btnShortcutSave = document.getElementById('btn-shortcut-save') as any;
const btnShortcutReset = document.getElementById('btn-shortcut-reset') as any;

// ---- 渲染 ----

function renderKeyboardChannels() {
  keyboardChannelsEl.innerHTML = '';
  for (const ch of SHORTCUT_CHANNELS) {
    const keys = shortcutState.keyboard[ch.key] || [];
    const isRecording = recordingGroup === 'keyboard' && recordingChannel === ch.key;

    const row = document.createElement('div');
    row.className = 'shortcut-row';

    // 标签 + 描述
    const info = document.createElement('div');
    info.className = 'shortcut-info';
    const labelSpan = document.createElement('span');
    labelSpan.className = 'shortcut-label';
    labelSpan.textContent = ch.label;
    const descSpan = document.createElement('span');
    descSpan.className = 'shortcut-desc';
    descSpan.textContent = ch.desc;
    info.append(labelSpan, descSpan);

    // 当前值显示
    const display = document.createElement('span');
    display.className = 'shortcut-display';
    display.dataset.channel = ch.key;
    if (isRecording) {
      display.textContent = '正在录制…';
      display.classList.add('recording');
    } else {
      display.textContent = formatShortcut(keys, shortcutState.platform);
    }

    // 录制按钮
    const recBtn = document.createElement('sl-button');
    recBtn.size = 'small';
    recBtn.variant = isRecording ? 'danger' : 'neutral';
    recBtn.textContent = isRecording ? '取消' : '录制';
    recBtn.addEventListener('click', () => {
      if (isRecording) {
        cancelRecording();
      } else {
        startRecording('keyboard', ch.key);
      }
    });

    // 清除按钮
    const clearBtn = document.createElement('sl-button');
    clearBtn.size = 'small';
    clearBtn.variant = 'default';
    clearBtn.textContent = '清除';
    clearBtn.addEventListener('click', () => {
      shortcutState.keyboard[ch.key] = [];
      renderKeyboardChannels();
    });

    const btnGroup = document.createElement('div');
    btnGroup.className = 'shortcut-btn-group';
    btnGroup.append(recBtn, clearBtn);

    row.append(info, display, btnGroup);
    keyboardChannelsEl.appendChild(row);
  }
}

function renderMouseChannels() {
  mouseChannelsEl.innerHTML = '';
  for (const ch of MOUSE_CHANNELS) {
    const current = shortcutState.mouse[ch.key] || 'none';

    const row = document.createElement('div');
    row.className = 'shortcut-row';

    const info = document.createElement('div');
    info.className = 'shortcut-info';
    const labelSpan = document.createElement('span');
    labelSpan.className = 'shortcut-label';
    labelSpan.textContent = ch.label;
    const descSpan = document.createElement('span');
    descSpan.className = 'shortcut-desc';
    descSpan.textContent = ch.desc;
    info.append(labelSpan, descSpan);

    const select = document.createElement('sl-select');
    select.className = 'mouse-select';
    select.size = 'small';
    select.value = current;
    select.addEventListener('sl-change', (e: any) => {
      const val = e.target.value;
      shortcutState.mouse[ch.key] = val === 'none' ? null : val;
    });

    const options = [
      { value: 'none', label: '无' },
      { value: 'forward', label: '前进键 (Forward)' },
      { value: 'back', label: '后退键 (Back)' },
    ];
    for (const opt of options) {
      const option = document.createElement('sl-option');
      option.value = opt.value;
      option.textContent = opt.label;
      select.appendChild(option);
    }

    row.append(info, select);
    mouseChannelsEl.appendChild(row);
  }
}

function renderAllShortcut() {
  renderKeyboardChannels();
  renderMouseChannels();
}

// ---- 录制逻辑 ----

function updateRecordingHint(mods: string[]) {
  const displayEl = document.querySelector(`.shortcut-display[data-channel="${recordingChannel}"]`);
  if (displayEl) {
    const names = mods.map(m => FAMILY_DISPLAY[shortcutState.platform]?.[m] ?? m).join(' + ');
    displayEl.textContent = `正在录制… ${names} + ?`;
  }
}

function cancelRecording() {
  recordingTeardown?.();
  recordingTeardown = null;
  recordingChannel = null;
  recordingGroup = null;
  renderKeyboardChannels();
}

function startRecording(group: 'keyboard', channel: string) {
  // 取消已有的录制
  if (recordingTeardown) recordingTeardown();

  recordingGroup = group;
  recordingChannel = channel;
  const platform = shortcutState.platform;

  // 点击面板外取消录制
  const onClickAway = (e: MouseEvent) => {
    const target = e.target as HTMLElement;
    if (!target.closest('#panel-shortcut')) {
      cancelRecording();
    }
  };
  document.addEventListener('click', onClickAway, true);

  if (platform === 'windows') {
    // Windows：追踪修饰键组合
    const heldMods: string[] = [];

    const onKeyDown = (e: KeyboardEvent) => {
      // Escape 取消
      if (e.code === 'Escape') { e.preventDefault(); cancelRecording(); return; }

      // 追踪修饰键家族
      if (e.code.startsWith('Control')) { e.preventDefault(); if (!heldMods.includes('Control')) heldMods.push('Control'); updateRecordingHint(heldMods); return; }
      if (e.code.startsWith('Meta'))     { e.preventDefault(); if (!heldMods.includes('Meta')) heldMods.push('Meta'); updateRecordingHint(heldMods); return; }
      if (e.code.startsWith('Alt'))      { e.preventDefault(); if (!heldMods.includes('Alt')) heldMods.push('Alt'); updateRecordingHint(heldMods); return; }
      if (e.code.startsWith('Shift'))    { e.preventDefault(); if (!heldMods.includes('Shift')) heldMods.push('Shift'); updateRecordingHint(heldMods); return; }

      // 非修饰键：确认组合
      e.preventDefault();
      const rdevName = DOM_CODE_TO_RDEV[e.code];
      if (rdevName && heldMods.length > 0) {
        // 修饰键 + 普通键 → 保存修饰键组合
        shortcutState.keyboard[channel] = heldMods.slice();
      } else if (rdevName) {
        // 无修饰键 → 单键
        shortcutState.keyboard[channel] = [rdevName];
      }
      stopRecording();
      renderKeyboardChannels();
    };

    const onKeyUp = (e: KeyboardEvent) => {
      if (e.code.startsWith('Control')) heldMods.splice(heldMods.indexOf('Control'), 1);
      if (e.code.startsWith('Meta'))    heldMods.splice(heldMods.indexOf('Meta'), 1);
      if (e.code.startsWith('Alt'))     heldMods.splice(heldMods.indexOf('Alt'), 1);
      if (e.code.startsWith('Shift'))   heldMods.splice(heldMods.indexOf('Shift'), 1);

      // 所有修饰键释放：如果只有修饰键被按下，以修饰键组合确认
      if (heldMods.length === 0) {
        // 等到下一个 tick 判断：如果没按其他键，可能是只按了修饰键
        // 这里不做自动确认，留给超时处理
      }
    };

    document.addEventListener('keydown', onKeyDown);
    document.addEventListener('keyup', onKeyUp);
    recordingTeardown = () => {
      document.removeEventListener('keydown', onKeyDown);
      document.removeEventListener('keyup', onKeyUp);
      document.removeEventListener('click', onClickAway, true);
    };
  } else {
    // macOS：捕获单个按键
    const onKeyDown = (e: KeyboardEvent) => {
      // Escape 取消
      if (e.code === 'Escape') { e.preventDefault(); cancelRecording(); return; }

      // 阻止修饰键默认行为
      if (e.code.startsWith('Meta') || e.code.startsWith('Alt') ||
          e.code.startsWith('Control') || e.code.startsWith('Shift')) {
        e.preventDefault();
      }

      const rdevName = DOM_CODE_TO_RDEV[e.code];
      if (rdevName) {
        shortcutState.keyboard[channel] = [rdevName];
        stopRecording();
        renderKeyboardChannels();
        return;
      }
      // 不支持的按键
      toast('danger', `暂不支持的按键: ${e.code}`);
    };

    document.addEventListener('keydown', onKeyDown);
    recordingTeardown = () => {
      document.removeEventListener('keydown', onKeyDown);
      document.removeEventListener('click', onClickAway, true);
    };
  }

  // 10 秒超时
  const timeoutId = setTimeout(() => {
    if (recordingChannel === channel) {
      cancelRecording();
      toast('danger', '录制超时，已取消');
    }
  }, 10000);
  const prevTeardown = recordingTeardown!;
  recordingTeardown = () => { clearTimeout(timeoutId); prevTeardown(); };

  renderKeyboardChannels();
}

function stopRecording() {
  recordingTeardown?.();
  recordingTeardown = null;
  recordingChannel = null;
  recordingGroup = null;
}

// ---- 事件 ----

btnShortcutSave.addEventListener('click', () => {
  btnShortcutSave.textContent = '保存中…';
  btnShortcutSave.disabled = true;
  emit('drop-typing://save-shortcut-config', {
    keyboard: shortcutState.keyboard,
    mouse: shortcutState.mouse,
  });
});

btnShortcutReset.addEventListener('click', async () => {
  const ok = await showConfirm('确认将快捷键重置为平台默认值？当前修改将丢失。');
  if (!ok) return;
  emit('drop-typing://reset-shortcut-config');
});

function requestShortcutConfig() {
  emit('drop-typing://get-shortcut-config');
}

// ---- 事件监听 ----

listen<any>('drop-typing://shortcut-config', (e) => {
  const d = e.payload;
  shortcutState.platform = d.platform;
  shortcutState.keyboard = { ...d.keyboard };
  shortcutState.mouse = { ...d.mouse };
  shortcutState.defaults = {
    keyboard: { ...d.defaults.keyboard },
    mouse: { ...d.defaults.mouse },
  };
  // 平台提示
  platformHintEl.textContent = d.platform === 'macos'
    ? 'macOS：单键快捷键（如右 Cmd、右 Opt 等）'
    : 'Windows：组合快捷键（如 Win + Alt 等）';
  renderAllShortcut();
});

listen<any>('drop-typing://shortcut-saved', (e) => {
  btnShortcutSave.textContent = '保存';
  btnShortcutSave.disabled = false;
  if (e.payload.success) {
    toast('success', '快捷键已保存');
    promptRestart('快捷键配置需要重启应用后才能生效，是否立即重启？');
  } else {
    toast('danger', e.payload.error || '保存失败');
  }
});

listen<any>('drop-typing://shortcut-reset', (e) => {
  if (e.payload.success) {
    requestShortcutConfig();
    toast('success', '已重置为默认值。请重启应用使配置生效。');
  } else {
    toast('danger', e.payload.error || '重置失败');
  }
});

// ── 高级面板（模型 / 毫秒 / 配置文件） ─────────────────────────────────

let generalConfig: any = null;
let generalLoaded = false;
let configFileLoaded = false;

function requestGeneralConfig() {
  emit('drop-typing://get-general-config');
}

function requestConfigFile() {
  emit('drop-typing://get-config-file');
}

// 子 Tab 切换
document.querySelectorAll<HTMLElement>('#advanced-tabs button').forEach(btn => {
  btn.addEventListener('click', () => {
    const sub = btn.dataset.sub!;
    document.querySelectorAll('#advanced-tabs button').forEach(b => b.classList.remove('active'));
    btn.classList.add('active');
    document.querySelectorAll('.sub-panel').forEach(p => p.classList.remove('active'));
    document.getElementById(`sub-${sub}`)!.classList.add('active');
  });
});

function fillGeneralForm() {
  if (!generalConfig) return;
  const a = generalConfig.asr || {};
  asrProvider.value = a.provider || '';
  asrProtocol.value = a.protocol || 'dashscope-realtime';
  asrModel.value = a.model || '';
  asrBaseUrl.value = a.base_url || '';
  asrApiKey.value = a.api_key || '';

  const l = generalConfig.llm || {};
  llmProvider.value = l.provider || '';
  llmProtocol.value = l.protocol || 'openai-chat';
  llmModel.value = l.model || '';
  llmBaseUrl.value = l.base_url || '';
  llmApiKey.value = l.api_key || '';
  llmStrength.value = l.strength || 'standard';

  const t = generalConfig.thresholds || {};
  millisLongPress.value = t.long_press_threshold_ms ?? 150;
  millisDoublePress.value = t.double_press_window_ms ?? 350;
  millisCommandCountdown.value = t.command_countdown_ms ?? 2000;
}

function collectGeneralPayload() {
  return {
    asr: {
      provider: asrProvider.value,
      protocol: asrProtocol.value,
      model: asrModel.value,
      base_url: asrBaseUrl.value,
      api_key: asrApiKey.value,
    },
    llm: {
      provider: llmProvider.value,
      protocol: llmProtocol.value,
      model: llmModel.value,
      base_url: llmBaseUrl.value,
      api_key: llmApiKey.value,
      strength: llmStrength.value,
    },
    thresholds: {
      long_press_threshold_ms: parseInt(millisLongPress.value, 10),
      double_press_window_ms: parseInt(millisDoublePress.value, 10),
      command_countdown_ms: parseInt(millisCommandCountdown.value, 10),
    },
  };
}

function saveGeneralConfig() {
  const payload = collectGeneralPayload();
  const thresholdKeys = [
    'long_press_threshold_ms',
    'double_press_window_ms',
    'command_countdown_ms',
  ] as const;
  for (const key of thresholdKeys) {
    const n = payload.thresholds[key];
    if (!Number.isFinite(n) || n < 50 || n > 10000) {
      toast('danger', `${key} 需为 50~10000 的整数`);
      return;
    }
  }
  btnGeneralSave.textContent = '保存中…';
  btnMillisSave.textContent = '保存中…';
  btnGeneralSave.disabled = true;
  btnMillisSave.disabled = true;
  emit('drop-typing://save-general-config', payload);
}

btnGeneralSave.addEventListener('click', saveGeneralConfig);
btnMillisSave.addEventListener('click', saveGeneralConfig);

function setTestButton(btn: any, busy: boolean, label = '测试连接') {
  btn.disabled = busy;
  btn.textContent = busy ? '测试中…' : label;
}

btnTestAsr.addEventListener('click', () => {
  setTestButton(btnTestAsr, true);
  emit('drop-typing://test-asr', { asr: collectGeneralPayload().asr });
});

btnTestLlm.addEventListener('click', () => {
  setTestButton(btnTestLlm, true);
  emit('drop-typing://test-llm', { llm: collectGeneralPayload().llm });
});

// 配置文件兜底编辑器
btnConfigReload.addEventListener('click', () => requestConfigFile());
btnConfigSave.addEventListener('click', () => {
  btnConfigSave.textContent = '保存中…';
  btnConfigSave.disabled = true;
  emit('drop-typing://save-config-file', { text: configFileText.value });
});

// 重启提示：所有需要重启的保存共用
function promptRestart(message?: string) {
  showConfirm(message || '部分配置需要重启应用后才能生效，是否立即重启？').then(ok => {
    if (ok) emit('drop-typing://restart');
  });
}

listen<any>('drop-typing://general-config', (e) => {
  generalConfig = e.payload;
  generalLoaded = true;
  fillGeneralForm();
});

listen<any>('drop-typing://config-file', (e) => {
  configFileText.value = e.payload.text || '';
  configFileLoaded = true;
});

listen<any>('drop-typing://config-file-saved', (e) => {
  btnConfigSave.textContent = '保存';
  btnConfigSave.disabled = false;
  if (e.payload.success) {
    toast('success', '配置文件已保存');
    // 文件可能改了任何段：刷新表单，避免双向编辑显示陈旧值
    requestGeneralConfig();
    requestCommandConfig();
    if (e.payload.restart_required) {
      promptRestart('配置中热键或唤醒词设置已变化，需要重启应用后才能生效，是否立即重启？');
    }
  } else {
    toast('danger', e.payload.error || '保存失败');
  }
});

listen<any>('drop-typing://restart-required', (e) => {
  promptRestart(e.payload.message || '部分配置需要重启应用后才能生效，是否立即重启？');
});

listen<any>('drop-typing://test-asr-result', (e) => {
  setTestButton(btnTestAsr, false);
  if (e.payload.success) toast('success', e.payload.message);
  else toast('danger', e.payload.message);
});

listen<any>('drop-typing://test-llm-result', (e) => {
  setTestButton(btnTestLlm, false);
  if (e.payload.success) toast('success', e.payload.message);
  else toast('danger', e.payload.message);
});

// 通用配置保存完成后恢复按钮状态（提示文案由既有 config-saved 监听负责）
listen<{ success: boolean; error?: string }>('drop-typing://config-saved', () => {
  btnGeneralSave.textContent = '保存';
  btnMillisSave.textContent = '保存';
  btnGeneralSave.disabled = false;
  btnMillisSave.disabled = false;
});

// ── 初始化 ────────────────────────────────────────────────────────────

emit('drop-typing://settings-ready');
