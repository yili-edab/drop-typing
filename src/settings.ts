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

const baseWrap = document.querySelector('#panel-base .editor-wrap')!;
const baseTextarea = document.getElementById('prompt-base') as HTMLTextAreaElement;
const btnBaseReset = document.getElementById('btn-base-reset') as any;
const btnBaseSave = document.getElementById('btn-base-save') as any;
const btnBaseAi = document.getElementById('btn-base-ai') as any;

const styleTabsContainer = document.getElementById('style-tabs')!;
const btnAddStyle = document.getElementById('btn-add-style') as any;
const styleWrap = document.querySelector('#panel-advanced .editor-wrap')!;
const styleTextarea = document.getElementById('style-textarea') as HTMLTextAreaElement;
const btnStyleReset = document.getElementById('btn-style-reset') as any;
const btnStyleSave = document.getElementById('btn-style-save') as any;
const btnStyleAi = document.getElementById('btn-style-ai') as any;

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

emit('drop-typing://settings-ready');
