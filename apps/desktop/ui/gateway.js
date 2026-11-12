function formatModelDisplayName(id) {
  if (id === 'gemini-3.8-flash-high') return 'Gemini 3.8 Flash';
  if (id === 'gemini-3-flash') return 'Gemini 3 Flash';
  if (id === 'gemini-3.1-flash-lite') return 'Gemini 3.1 Flash Lite';
  if (id === 'gemini-3.5-flash-lite') return 'Gemini 3.5 Flash Lite';
  return id;
}

const status = document.getElementById('status');
const start = document.getElementById('start');
const stop = document.getElementById('stop');
let generation = 0;
let discoveryGeneration = 0;
let verificationGeneration = 0;
let gatewayReady = false;
const accountStates = new Map();

const tabs = [...document.querySelectorAll('[role="tab"]')];
function selectTab(tab, focus = false) {
  for (const item of tabs) {
    const selected = item === tab;
    item.setAttribute('aria-selected', String(selected));
    item.tabIndex = selected ? 0 : -1;
    document.getElementById(item.getAttribute('aria-controls')).hidden = !selected;
  }
  document.querySelector('.page-heading h2').textContent = tab.textContent === '總覽' ? '閘道總覽' : tab.textContent;
  if (focus) tab.focus();
}
for (const tab of tabs) {
  tab.addEventListener('click', () => selectTab(tab));
  tab.addEventListener('keydown', event => {
    const index = tabs.indexOf(tab);
    let next;
    if (event.key === 'ArrowRight') next = (index + 1) % tabs.length;
    else if (event.key === 'ArrowLeft') next = (index + tabs.length - 1) % tabs.length;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = tabs.length - 1;
    else return;
    event.preventDefault();
    selectTab(tabs[next], true);
  });
}

// 頂部狀態燈與指標元素
const pulseIndicator = document.getElementById('pulse-indicator');
const quickStatusText = document.getElementById('quick-status-text');
const metricPort = document.getElementById('metric-port');
const metricStatusHint = document.getElementById('metric-status-hint');
const metricDesktop = document.getElementById('metric-desktop');
const metricDesktopHint = document.getElementById('metric-desktop-hint');
const metricAccounts = document.getElementById('metric-accounts');
const metricModels = document.getElementById('metric-models');

function showStoppedIndicators() {
  if (pulseIndicator) pulseIndicator.className = 'pulse-dot';
  if (quickStatusText) quickStatusText.textContent = '🔴 已停止';
  if (metricPort) metricPort.textContent = '未監聽';
  if (metricStatusHint) metricStatusHint.textContent = '閘道未運行 (憑證安全留存)';
}

async function appendLog(level, message, details = "") {
  const ts = new Date().toLocaleTimeString();
  const logBox = document.getElementById('activity-log');
  const detailsStr = typeof details === 'object' && details !== null ? JSON.stringify(details) : String(details);
  if (logBox) {
    const entry = `[${ts}] [${level}] ${message}${detailsStr ? ' - ' + detailsStr : ''}\n`;
    logBox.textContent += entry;
    logBox.scrollTop = logBox.scrollHeight;
  }
  try {
    await window.__TAURI__.core.invoke('log_message', { level, message, details: detailsStr });
  } catch (_) {
    if (logBox) logBox.textContent += `[${ts}] [ERROR] 無法寫入本機日誌檔，請檢查應用程式資料目錄權限。\n`;
  }
}

const clearLogBtn = document.getElementById('clear-log');
if (clearLogBtn) {
  clearLogBtn.addEventListener('click', () => {
    const logBox = document.getElementById('activity-log');
    if (logBox) logBox.textContent = '';
  });
}

const copyLogBtn = document.getElementById('copy-log-btn');
if (copyLogBtn) {
  copyLogBtn.addEventListener('click', async () => {
    const logBox = document.getElementById('activity-log');
    if (logBox && logBox.textContent) {
      try {
        await navigator.clipboard.writeText(logBox.textContent);
        const originalText = copyLogBtn.textContent;
        copyLogBtn.textContent = '✅ 已複製！';
        setTimeout(() => { copyLogBtn.textContent = originalText; }, 2000);
      } catch (_) {}
    }
  });
}

function renderAccounts() {
  let credentials = 0;
  let known = 0;
  for (const account of document.querySelectorAll('.account')) {
    const data = accountStates.get(account.dataset.provider) || {};
    const hasCred = data.hasCredential !== undefined ? data.hasCredential : data.has_credential;
    if (typeof hasCred === 'boolean') known++;
    if (hasCred) credentials++;
    const flow = ['authorizing', 'awaitingConfirmation', 'awaiting_confirmation', 'expired', 'error'].includes(data.state);
    for (const button of account.querySelectorAll('button')) {
      const action = button.dataset.action;
      let allowed = false;
      if (action === 'begin') allowed = (data.canStart !== undefined ? data.canStart : (hasCred === false && !flow));
      else if (action === 'renew') allowed = (data.canRenew !== undefined ? data.canRenew : (hasCred === true && !flow));
      else if (action === 'poll') allowed = true;
      else if (action === 'confirm') allowed = (data.canConfirm !== undefined ? data.canConfirm : (data.state === 'awaitingConfirmation' || data.state === 'awaiting_confirmation'));
      else if (action === 'cancel') allowed = (data.canCancel !== undefined ? data.canCancel : flow);
      else if (action === 'verify') allowed = (hasCred === true && !flow);
      button.disabled = !gatewayReady || !!data.busy || !allowed;
    }
  }
  if (metricAccounts) metricAccounts.textContent = known ? (credentials + ' 個本機憑證' + (known < 2 ? '（部分未確認）' : '')) : '未確認';
  const selected = document.querySelector('.account[data-provider="' + document.getElementById('account-type').value + '"]');
  if (selected) {
    const addBtn = document.getElementById('add-account');
    if (addBtn) addBtn.disabled = selected.querySelector('[data-action="begin"]').disabled;
  }
}

async function refreshAccounts(current) {
  for (const account of document.querySelectorAll('.account')) {
    const provider = account.dataset.provider;
    try {
      const res = await window.__TAURI__.core.invoke('oauth_action', { provider, action: 'poll' });
      if (current === generation) {
        if (res && typeof res === 'object') {
          accountStates.set(provider, res);
          account.querySelector('p').textContent = res.message || '';
        } else {
          account.querySelector('p').textContent = String(res);
        }
      }
    } catch (_) {
      if (current === generation) {
        account.querySelector('p').textContent = '無法檢查憑證狀態；請稍後重試';
      }
    }
  }
  renderAccounts();
}

function resetAccounts() {
  gatewayReady = false;
  accountStates.clear();
  ++discoveryGeneration;
  if (metricAccounts) metricAccounts.textContent = '未確認';
  if (metricModels) metricModels.textContent = '尚未查詢';
  document.getElementById('model-list').replaceChildren();
  document.getElementById('model-status').textContent = '';
  const select = document.getElementById('verify-model-select');
  if (select) {
    select.replaceChildren();
    const opt = document.createElement('option');
    opt.value = '';
    opt.textContent = '-- 請先查詢模型 --';
    select.appendChild(opt);
  }
  const verifyBtn = document.getElementById('verify-model');
  if (verifyBtn) verifyBtn.disabled = true;
  const verifyStatus = document.getElementById('verify-status');
  if (verifyStatus) verifyStatus.textContent = '';
  for (const account of document.querySelectorAll('.account')) {
    account.querySelector('p').textContent = '閘道已停止；本機憑證保留，登入狀態未確認';
  }
  renderAccounts();
  appendLog('INFO', '帳號與模型清單狀態已重置');
}

async function monitor(current, startError = null) {
  if (current !== generation) return;
  try {
    const result = await window.__TAURI__.core.invoke('gateway_status');
    if (current !== generation) return;
    const clearGuardBtn = document.getElementById('clear-guard');
    if (result.state === 'cleanup_pending') {
      if (clearGuardBtn) clearGuardBtn.style.display = 'none';
      status.textContent = '閘道已停止，但清理未完成；請再次按停止閘道';
      if (pulseIndicator) pulseIndicator.className = 'pulse-dot';
      if (quickStatusText) quickStatusText.textContent = '⚠️ 清理未完成';
      stop.disabled = false;
      start.disabled = true;
      return;
    }
    if (result.state === 'detached') {
      if (clearGuardBtn) clearGuardBtn.style.display = 'inline-block';
      status.textContent = `背景閘道仍在執行（127.0.0.1:${result.port}），此視窗無法接管。請先檢查舊閘道，手動停止對應程序後清除紀錄，再啟動閘道。`;
      if (quickStatusText) quickStatusText.textContent = '⚠️ 需重新連接';
      if (metricPort) metricPort.textContent = `127.0.0.1:${result.port}`;
      if (metricStatusHint) metricStatusHint.textContent = '背景程序狀態待確認';
      for (const account of document.querySelectorAll('.account')) account.querySelector('p').textContent = '背景閘道尚未接管，登入狀態未知';
      stop.disabled = true;
      start.disabled = true;
      return;
    }
    if (result.state === 'needs_recovery') {
      if (clearGuardBtn) clearGuardBtn.style.display = 'inline-block';
      status.textContent = '發現上次啟動紀錄，請先檢查舊閘道';
      if (pulseIndicator) pulseIndicator.className = 'pulse-dot';
      if (quickStatusText) quickStatusText.textContent = '⚠️ 需檢查紀錄';
      stop.disabled = true;
      start.disabled = true;
      for (const account of document.querySelectorAll('.account')) account.querySelector('p').textContent = '上次登入資料狀態未知，請先檢查舊閘道';
      return;
    }
    if (clearGuardBtn) clearGuardBtn.style.display = 'none';
    if (result.state === 'exited' || result.state === 'stopped') {
      status.textContent = result.state === 'exited' ? '閘道意外停止，請重新啟動' : (startError ? `啟動失敗：${startError}` : '閘道已停止 (本機登入憑證已安全保留)');
      showStoppedIndicators();
      stop.disabled = true;
      start.disabled = false;
      resetAccounts();
      return;
    }
    if (result.state === 'degraded') {
      gatewayReady = true;
      status.textContent = '閘道無法回應健康檢查：127.0.0.1:' + result.port;
      if (pulseIndicator) pulseIndicator.className = 'pulse-dot degraded';
      if (quickStatusText) quickStatusText.textContent = '🟡 回應降級 (埠號 ' + result.port + ')';
      if (metricPort) metricPort.textContent = '127.0.0.1:' + result.port;
      if (metricStatusHint) metricStatusHint.textContent = '閘道健康檢查異常或逾時';
      stop.disabled = false;
      start.disabled = true;
      renderAccounts();
      return;
    }
    status.textContent = result.state === 'healthy'
      ? (result.adopted ? '已安全接管背景閘道：127.0.0.1:' + result.port : '閘道健康運作中：127.0.0.1:' + result.port)
      : `閘道無法回應健康檢查：127.0.0.1:${result.port}`;
    if (result.state === 'healthy') {
      gatewayReady = true;
      if (result.adopted) {
        appendLog('SUCCESS', '已安全接管背景閘道');
        void refreshAccounts(current);
      }
      if (pulseIndicator) pulseIndicator.className = 'pulse-dot running';
      if (quickStatusText) quickStatusText.textContent = `🟢 運作中 (埠號 ${result.port})`;
      if (metricPort) metricPort.textContent = `127.0.0.1:${result.port}`;
      if (metricStatusHint) metricStatusHint.textContent = '閘道健康檢查通過';
      stop.disabled = false;
      start.disabled = true;
      renderAccounts();
    }
  } catch (_) {
    if (current !== generation) return;
    status.textContent = '無法檢查閘道狀態';
    if (pulseIndicator) pulseIndicator.className = 'pulse-dot';
    if (quickStatusText) quickStatusText.textContent = '⚪️ 檢查失敗';
  }
  if (current === generation) setTimeout(() => monitor(current), 2000);
}

start.addEventListener('click', async () => {
  const current = ++generation;
  start.disabled = true;
  status.textContent = '啟動中…';
  appendLog('INFO', '正在啟動閘道與側車程序…');
  try {
    const port = await window.__TAURI__.core.invoke('start_gateway');
    status.textContent = `閘道健康運作中：127.0.0.1:${port}`;
    appendLog('SUCCESS', `閘道啟動成功，監聽 127.0.0.1:${port}`);
    try {
      const customPath = document.getElementById('codex-path-input').value.trim() || null;
      if (await window.__TAURI__.core.invoke('codex_resume', { customPath })) {
        appendLog('SUCCESS', '已恢復 ModelBridge 供應商與模型設定');
      }
    } catch (error) {
      appendLog('ERROR', '閘道已啟動，但無法恢復供應商設定', error);
      const info = document.getElementById('inspection');
      if (info) info.textContent = `閘道已啟動，但無法恢復供應商設定：${error}`;
    }
    await refreshAccounts(current);
    stop.disabled = false;
    monitor(current);
    await checkCodexConfigStatus();
  } catch (error) {
    status.textContent = `啟動失敗：${error}`;
    appendLog('ERROR', '閘道啟動失敗', error);
    await monitor(current, error);
  }
});

const stopConfirmation = document.getElementById('stop-confirmation');
stop.addEventListener('click', () => { stopConfirmation.hidden = false; });
document.getElementById('stop-cancel').addEventListener('click', () => { stopConfirmation.hidden = true; });
document.getElementById('stop-confirm').addEventListener('click', async () => {
  stopConfirmation.hidden = true;
  ++generation;
  stop.disabled = true;
  status.textContent = '停止中…';
  appendLog('INFO', '正在停止閘道程序…');
  try {
    await window.__TAURI__.core.invoke('stop_gateway');
    showStoppedIndicators();
    let switchMessage = '';
    try {
      const customPath = document.getElementById('codex-path-input').value.trim() || null;
      await appendLog('INFO', '閘道已停止，開始切換 Codex 原生設定', { configPath: customPath || '預設路徑' });
      const changed = await window.__TAURI__.core.invoke('codex_suspend', { customPath });
      switchMessage = changed ? '；已還原停用前的供應商與模型設定' : '；未發現由 ModelBridge 管理的設定';
      await appendLog(changed ? 'SUCCESS' : 'INFO', changed ? 'Codex 原生設定切換完成' : 'Codex 設定未受 ModelBridge 管理', { configPath: customPath || '預設路徑' });
    } catch (error) {
      switchMessage = `；無法自動切回原生設定：${error}`;
      await appendLog('ERROR', '閘道已停止，但 Codex 原生設定切換失敗', { error: String(error), configPath: document.getElementById('codex-path-input').value.trim() || '預設路徑' });
    }
    status.textContent = `閘道已停止 (本機登入憑證已安全保留)${switchMessage}`;
    appendLog('SUCCESS', '閘道已成功停止，本地登入憑證完整保留在 runtime/auth/');
    resetAccounts();
    start.disabled = false;
  } catch (error) {
    status.textContent = `停止失敗：${error}`;
    appendLog('ERROR', '閘道停止失敗', error);
    stop.disabled = false;
    monitor(generation);
  }
});

document.getElementById('inspect').addEventListener('click', async () => {
  const result = document.getElementById('inspection');
  result.textContent = '檢查中…';
  try {
    const res = await window.__TAURI__.core.invoke('inspect_previous_gateway');
    result.textContent = res;
    appendLog('INFO', '檢查上次啟動紀錄結果', res);
  } catch (error) {
    result.textContent = `檢查失敗：${error}`;
    appendLog('ERROR', '檢查上次啟動紀錄失敗', error);
  }
});

const clearGuardBtn = document.getElementById('clear-guard');
if (clearGuardBtn) {
  clearGuardBtn.addEventListener('click', async () => {
    const result = document.getElementById('inspection');
    result.textContent = '正在清理舊保護紀錄…';
    clearGuardBtn.disabled = true;
    try {
      const msg = await window.__TAURI__.core.invoke('clear_run_guard');
      result.textContent = msg;
      appendLog('SUCCESS', '清除保護紀錄成功', msg);
      clearGuardBtn.style.display = 'none';
      await monitor(generation);
    } catch (error) {
      result.textContent = `清理失敗：${error}`;
      appendLog('ERROR', '清除保護紀錄失敗', error);
      clearGuardBtn.disabled = false;
    }
  });
}

document.getElementById('add-account').addEventListener('click', () => {
  const provider = document.getElementById('account-type').value;
  const account = document.querySelector(`.account[data-provider="${provider}"]`);
  account.querySelector('[data-action="begin"]').click();
});

for (const account of document.querySelectorAll('.account')) {
  const result = account.querySelector('p');
  for (const button of account.querySelectorAll('button')) {
    button.addEventListener('click', async () => {
      const current = generation;
      const provider = account.dataset.provider;
      const action = button.dataset.action;
      const st = accountStates.get(provider) || {};
      st.busy = true;
      renderAccounts();
      result.textContent = '處理中…';
      appendLog('INFO', `帳號 [${provider}] 操作 [${action}] 開始`);
      try {
        let res;
        if (action === 'begin') {
          res = await window.__TAURI__.core.invoke('oauth_start', { provider, terms: document.getElementById('terms').checked, renew: false });
        } else if (action === 'renew') {
          res = await window.__TAURI__.core.invoke('oauth_start', { provider, terms: document.getElementById('terms').checked, renew: true });
        } else {
          res = await window.__TAURI__.core.invoke('oauth_action', { provider, action });
        }
        if (current === generation) {
          if (res && typeof res === 'object') {
            accountStates.set(provider, res);
            result.textContent = res.message || '';
          } else {
            result.textContent = String(res);
          }
          appendLog('SUCCESS', '帳號 [' + provider + '] 操作 [' + action + '] 結果', res);
        }
      } catch (error) {
        if (current === generation) result.textContent = `操作未完成：${error}`;
        appendLog('ERROR', `帳號 [${provider}] 操作 [${action}] 失敗`, error);
      } finally {
        const finalSt = accountStates.get(provider) || {};
        finalSt.busy = false;
        renderAccounts();
      }
    });
  }
}

async function initializeGateway() {
  try {
    if (await window.__TAURI__.core.invoke('auto_start_enabled')) {
      const result = await window.__TAURI__.core.invoke('gateway_status');
      if (result.state === 'stopped') {
        start.disabled = false;
        start.click();
        return;
      }
    }
  } catch (error) {
    appendLog('ERROR', '自動啟動閘道失敗', error);
  }
  monitor(generation);
}
initializeGateway();

document.getElementById('model-provider').addEventListener('change', () => {
  ++discoveryGeneration;
  if (metricModels) metricModels.textContent = '尚未查詢';
  document.getElementById('model-list').replaceChildren();
  document.getElementById('model-status').textContent = '';
  const select = document.getElementById('verify-model-select');
  if (select) {
    select.replaceChildren();
    const opt = document.createElement('option');
    opt.value = '';
    opt.textContent = '-- 請先查詢模型 --';
    select.appendChild(opt);
  }
  const verifyBtn = document.getElementById('verify-model');
  if (verifyBtn) verifyBtn.disabled = true;
  const verifyStatus = document.getElementById('verify-status');
  if (verifyStatus) verifyStatus.textContent = '';
});

document.getElementById('discover-models').addEventListener('click', async () => {
  const current = generation;
  const query = ++discoveryGeneration;
  const button = document.getElementById('discover-models');
  const message = document.getElementById('model-status');
  const list = document.getElementById('model-list');
  const select = document.getElementById('verify-model-select');
  const verifyBtn = document.getElementById('verify-model');
  const provider = document.getElementById('model-provider').value;
  button.disabled = true;
  if (metricModels) metricModels.textContent = '尚未查詢';
  list.replaceChildren();
  if (select) {
    select.replaceChildren();
    const opt = document.createElement('option');
    opt.value = '';
    opt.textContent = '-- 請先查詢模型 --';
    select.appendChild(opt);
  }
  if (verifyBtn) verifyBtn.disabled = true;
  message.textContent = '查詢中…';
    appendLog('INFO', `正在向閘道查詢 [${provider}] 之模型清單…`);
  try {
    const models = await window.__TAURI__.core.invoke('discovered_models', { provider });
    if (current !== generation || query !== discoveryGeneration) return;
    if (select && models.length > 0) {
      select.replaceChildren();
      for (const model of models) {
        const opt = document.createElement('option');
        opt.value = model;
        const optDisp = formatModelDisplayName(model);
        opt.textContent = optDisp !== model ? `${optDisp} (${model})` : model;
        select.appendChild(opt);
      }
      if (verifyBtn) verifyBtn.disabled = false;
    }
    for (const model of models) {
      const item = document.createElement('li');
      item.id = `model-entry-${model.replace(/[^a-zA-Z0-9_-]/g, '_')}`;
      const isGemini = model.startsWith('gemini');
      const badgeIcon = isGemini ? '✨' : '⚡️';
      const disp = formatModelDisplayName(model);
      item.textContent = `${badgeIcon} ${disp} [${model}]（已發現，尚未驗證）`;
      list.appendChild(item);
    }
    message.textContent = models.length ? `已發現 ${models.length} 個模型，尚未驗證` : '目前沒有可顯示的模型';
    if (metricModels) {
      metricModels.textContent = `${models.length} 款模型`;
    }
    appendLog('SUCCESS', `查詢 [${provider}] 模型成功，發現 ${models.length} 個尚未驗證的模型`, models);
  } catch (error) {
    if (current === generation && query === discoveryGeneration) message.textContent = `查詢未完成：${error}`;
    appendLog('ERROR', `查詢 [${provider}] 模型失敗`, error);
  } finally {
    button.disabled = false;
  }
});

const verifyBtn = document.getElementById('verify-model');
if (verifyBtn) {
  verifyBtn.addEventListener('click', async () => {
    const current = generation;
    const currentDiscovery = discoveryGeneration;
    const consent = document.getElementById('quota-consent').checked;
    const verifyStatus = document.getElementById('verify-status');
    const select = document.getElementById('verify-model-select');
    const model = select.value;
    const provider = document.getElementById('model-provider').value;
    if (!consent) {
      verifyStatus.textContent = '操作已中止：未取得模型額度消耗同意核取。';
      appendLog('WARN', 'Gate 3 操作已中止：未勾選額度消耗同意');
      return;
    }
    if (!model) {
      verifyStatus.textContent = '請先選擇欲驗證的模型。';
      return;
    }
    verifyBtn.disabled = true;
    verifyStatus.textContent = `正在對 ${model} 發起真實串流雙回合驗證（可能消耗帳號額度）…`;
    appendLog('INFO', `Gate 3 驗證開始：模型 [${model}] (來源: ${provider})，已取得額度同意`);
    try {
      const result = await window.__TAURI__.core.invoke('verify_model_roundtrip', {
        provider,
        model,
        quotaConfirmed: consent
      });
      if (current !== generation || currentDiscovery !== discoveryGeneration) return;
      verifyStatus.textContent = `驗證成功：${result}`;
      appendLog('SUCCESS', `Gate 3 驗證通過：模型 [${model}]`, result);
      const account = document.querySelector('.account[data-provider="' + provider + '"]');
      if (account) account.querySelector('p').textContent = '本次模型測試通過，追蹤標頭符合該憑證；不保證帳號身分或後續持續有效';
      const entry = document.getElementById(`model-entry-${model.replace(/[^a-zA-Z0-9_-]/g, '_')}`);
      if (entry) {
        entry.textContent = `✓ ${model}（本次驗證通過）`;
        entry.style.fontWeight = 'bold';
        entry.style.color = 'var(--success, #15765d)';
      }
      const codexModelInput = document.getElementById('codex-model-input');
      if (codexModelInput && !codexModelInput.value) {
        codexModelInput.value = model;
      }
    } catch (error) {
      if (current !== generation || currentDiscovery !== discoveryGeneration) return;
      verifyStatus.textContent = `驗證失敗：${error}`;
      const account = document.querySelector('.account[data-provider="' + provider + '"]');
      if (account) account.querySelector('p').textContent = '本次模型驗證未通過；請查看模型管理的錯誤訊息';
      appendLog('ERROR', `Gate 3 驗證失敗：模型 [${model}]`, error);
    } finally {
      if (current === generation && currentDiscovery === discoveryGeneration) {
        verifyBtn.disabled = false;
      }
    }
  });
}

// --- Codex & ChatGPT Desktop Config Adapter ---
const codexPathInput = document.getElementById('codex-path-input');
const codexCatalogInput = document.getElementById('codex-catalog-input');
const codexDetectInfo = document.getElementById('codex-detect-info');
const codexModelInput = document.getElementById('codex-model-input');
const codexAllowOverride = document.getElementById('codex-allow-override');
const codexCheckStatusBtn = document.getElementById('codex-check-status');
const codexSyncAuthBtn = document.getElementById('codex-sync-auth-btn');
const codexSyncCatalogBtn = document.getElementById('codex-sync-catalog-btn');
const codexPreviewDiffBtn = document.getElementById('codex-preview-diff');
const codexApplyBtn = document.getElementById('codex-apply-btn');
const codexRollbackBtn = document.getElementById('codex-rollback-btn');
const codexStatusMsg = document.getElementById('codex-status-msg');
const codexPreviewBox = document.getElementById('codex-preview-box');
const codexDiffOutput = document.getElementById('codex-diff-output');
const oneClickSetupBtn = document.getElementById('one-click-setup-btn');

async function detectCodexPath() {
  try {
    const det = await window.__TAURI__.core.invoke('codex_detect');
    if (codexPathInput && !codexPathInput.value) {
      codexPathInput.value = det.cli_config_path;
    }
    if (codexCatalogInput && !codexCatalogInput.value) {
      codexCatalogInput.value = det.cli_config_path.replace(/config\.toml$/, 'model_catalog.json');
    }
    if (codexDetectInfo) {
      codexDetectInfo.textContent = `偵測依據: ${det.basis} | 提供者對齊: Custom`;
    }
    await checkCodexConfigStatus();
  } catch (err) {
    if (codexDetectInfo) codexDetectInfo.textContent = `路徑偵測失敗: ${err}`;
  }
}

async function checkCodexConfigStatus() {
  const path = codexPathInput ? codexPathInput.value.trim() : "";
  if (!path) return;
  try {
    const res = await window.__TAURI__.core.invoke('codex_status', { customPath: path });
    let desc = `檔案存在: ${res.exists ? '是' : '否'}`;
    if (res.is_symlink) desc += ' (符號連結)';
    if (res.managed_by_us) {
      desc += ' | 狀態: 由 ModelBridge (Custom 提供者) 管理中';
      if (res.external_edit_detected) desc += ' [警告：偵測到外部修改]';
      if (res.has_backup) desc += ' (有備份檔可還原)';
      if (res.current_model) desc += ` | 目前模型: ${res.current_model}`;
    } else if (res.exists) {
      desc += ' | 狀態: 未由本閘道管理 (原生或外部設定)';
    }

    if (codexStatusMsg) {
      codexStatusMsg.style.display = 'flex';
      codexStatusMsg.textContent = desc;
      codexStatusMsg.style.color = res.external_edit_detected ? '#ef4444' : (res.managed_by_us ? '#10b981' : 'var(--text-secondary)');
    }

    if (metricDesktop) {
      metricDesktop.textContent = res.managed_by_us ? 'Custom' : (res.exists ? '外部設定' : '未設定');
      metricDesktop.style.color = res.managed_by_us ? '#10b981' : '#f59e0b';
    }
    if (metricDesktopHint) {
      metricDesktopHint.textContent = res.current_model ? `預設: ${res.current_model}` : '閘道使用自己的 OAuth';
    }

    appendLog('INFO', `檢查 Codex 設定狀態: ${path}`, desc);
  } catch (err) {
    if (codexStatusMsg) {
      codexStatusMsg.style.display = 'flex';
      codexStatusMsg.textContent = `檢查狀態失敗: ${err}`;
      codexStatusMsg.style.color = '#ef4444';
    }
    appendLog('ERROR', '檢查 Codex 設定失敗', err);
  }
}

// ⚡️ 一鍵最佳化整合 (One-Click Setup)
if (oneClickSetupBtn) {
  oneClickSetupBtn.addEventListener('click', async () => {
    oneClickSetupBtn.disabled = true;
    const oldText = oneClickSetupBtn.textContent;
    oneClickSetupBtn.textContent = '⏳ 正在執行一鍵整合…';
    if (codexStatusMsg) {
      codexStatusMsg.style.display = 'flex';
      codexStatusMsg.textContent = '🚀 [1/2] 正在同步模型清單至 model_catalog.json…';
      codexStatusMsg.style.color = '#2563eb';
    }
    appendLog('INFO', '開始一鍵設定 ChatGPT Desktop 整合流程…');

    try {
      const customPath = codexPathInput ? codexPathInput.value.trim() : "";
      const customCatalogPath = codexCatalogInput ? codexCatalogInput.value.trim() : "";
      const model = codexModelInput ? codexModelInput.value.trim() : null;
      const allowOverride = codexAllowOverride ? codexAllowOverride.checked : true;

      // 步驟 1: 同步模型目錄
      let summary = null;
      try {
        summary = await window.__TAURI__.core.invoke('sync_model_catalog', {
          customCatalogPath: customCatalogPath || null
        });
        appendLog('SUCCESS', `[1/2] 模型目錄同步完成 (總計 ${summary.total_models} 筆)`);
      } catch (catErr) {
        appendLog('WARN', `[1/2] 模型清單同步提示: ${catErr}`);
      }

      // 步驟 2: 套用 Custom 閘道端點至 config.toml。不讀寫登入檔。
      if (codexStatusMsg) codexStatusMsg.textContent = '🚀 [2/2] 正在套用 Custom 閘道端點至 config.toml…';
      const applyMsg = await window.__TAURI__.core.invoke('codex_apply', {
        customPath: customPath || null,
        model: model || null,
        allowOverride: allowOverride,
        customCatalogPath: customCatalogPath || null
      });
      appendLog('SUCCESS', `[2/2] 套用閘道設定成功: ${applyMsg}`);

      const catalogMessage = summary ? `模型目錄共有 ${summary.total_models} 筆。` : '模型目錄同步未完成，請查看日誌。';
      const doneMsg = `✅ 已套用 Custom 供應商端點。${catalogMessage}閘道使用自己的 OAuth，沒有讀寫 Codex 登入檔。`;
      if (codexStatusMsg) {
        codexStatusMsg.textContent = doneMsg;
        codexStatusMsg.style.color = '#10b981';
      }
      appendLog('SUCCESS', doneMsg);
      await checkCodexConfigStatus();
    } catch (err) {
      if (codexStatusMsg) {
        codexStatusMsg.textContent = `❌ 一鍵整合中斷：${err}`;
        codexStatusMsg.style.color = '#ef4444';
      }
      appendLog('ERROR', '一鍵整合失敗', err);
    } finally {
      oneClickSetupBtn.disabled = false;
      oneClickSetupBtn.textContent = oldText;
    }
  });
}

if (codexCheckStatusBtn) {
  codexCheckStatusBtn.addEventListener('click', checkCodexConfigStatus);
}

if (codexSyncAuthBtn) {
  codexSyncAuthBtn.addEventListener('click', async () => {
    if (codexStatusMsg) {
      codexStatusMsg.style.display = 'flex';
      codexStatusMsg.textContent = '正在預覽登入隔離…';
      codexStatusMsg.style.color = '#2563eb';
    }
    try {
      const preview = await window.__TAURI__.core.invoke('preview_chatgpt_auth');
      const accepted = window.confirm(`${preview.message}\n\n不會複製 refresh token。要執行這個明確動作嗎？`);
      if (!accepted) {
        if (codexStatusMsg) codexStatusMsg.textContent = '已取消。登入檔沒有變更。';
        appendLog('INFO', '已取消登入隔離確認');
        return;
      }
      const summary = await window.__TAURI__.core.invoke('confirm_chatgpt_auth');
      if (codexStatusMsg) {
        codexStatusMsg.textContent = summary.message;
        codexStatusMsg.style.color = '#10b981';
      }
      appendLog('SUCCESS', '已確認登入隔離', summary);
    } catch (err) {
      if (codexStatusMsg) {
        codexStatusMsg.textContent = `登入隔離失敗: ${err}`;
        codexStatusMsg.style.color = '#ef4444';
      }
      appendLog('ERROR', '登入隔離失敗', err);
    }
  });
}

if (codexSyncCatalogBtn) {
  codexSyncCatalogBtn.addEventListener('click', async () => {
    const customCatalogPath = codexCatalogInput ? codexCatalogInput.value.trim() : "";
    if (codexStatusMsg) {
      codexStatusMsg.style.display = 'flex';
      codexStatusMsg.textContent = '正在同步模型清單至 ChatGPT Desktop (model_catalog.json)…';
      codexStatusMsg.style.color = '#2563eb';
    }
    try {
      const summary = await window.__TAURI__.core.invoke('sync_model_catalog', {
        customCatalogPath: customCatalogPath || null
      });
      const desc = `已成功同步至 ${summary.catalog_path}！總模型數: ${summary.total_models} (新增: ${summary.added_slugs.length} 款，保留: ${summary.existing_slugs.length} 款${summary.backup_path ? '，已備份' : ''})`;
      if (codexStatusMsg) {
        codexStatusMsg.textContent = desc;
        codexStatusMsg.style.color = '#10b981';
      }
      appendLog('SUCCESS', '同步模型清單至 ChatGPT Desktop 完成', summary);
    } catch (err) {
      if (codexStatusMsg) {
        codexStatusMsg.textContent = `同步失敗: ${err}`;
        codexStatusMsg.style.color = '#ef4444';
      }
      appendLog('ERROR', '同步模型清單至 ChatGPT Desktop 失敗', err);
    }
  });
}

if (codexPreviewDiffBtn) {
  codexPreviewDiffBtn.addEventListener('click', async () => {
    const customPath = codexPathInput ? codexPathInput.value.trim() : "";
    const model = codexModelInput ? codexModelInput.value.trim() : null;
    const allowOverride = codexAllowOverride ? codexAllowOverride.checked : false;
    if (codexStatusMsg) {
      codexStatusMsg.style.display = 'flex';
      codexStatusMsg.textContent = '正在計算安全 Diff 預覽…';
      codexStatusMsg.style.color = '#2563eb';
    }
    try {
      const customCatalogPath = codexCatalogInput ? codexCatalogInput.value.trim() : "";
      const preview = await window.__TAURI__.core.invoke('codex_preview', {
        customPath: customPath || null,
        model: model || null,
        allowOverride: allowOverride,
        customCatalogPath: customCatalogPath || null
      });
      if (codexPreviewBox && codexDiffOutput) {
        codexPreviewBox.style.display = 'block';
        codexDiffOutput.textContent = preview.diff || "(無變更)";
      }
      if (codexStatusMsg) {
        codexStatusMsg.textContent = preview.creates_file ? '預覽完成：此操作將建立全新設定檔' : '預覽完成：已計算安全差異（機密金鑰已自動遮蔽保護）';
        codexStatusMsg.style.color = '#10b981';
      }
      appendLog('INFO', '預覽 Codex 設定安全 Diff 完成', { creates_file: preview.creates_file });
    } catch (err) {
      if (codexStatusMsg) {
        codexStatusMsg.textContent = `預覽失敗: ${err}`;
        codexStatusMsg.style.color = '#ef4444';
      }
      appendLog('ERROR', '預覽 Codex 設定失敗', err);
    }
  });
}

if (codexApplyBtn) {
  codexApplyBtn.addEventListener('click', async () => {
    const customPath = codexPathInput ? codexPathInput.value.trim() : "";
    const model = codexModelInput ? codexModelInput.value.trim() : null;
    const allowOverride = codexAllowOverride ? codexAllowOverride.checked : false;
    if (codexStatusMsg) {
      codexStatusMsg.style.display = 'flex';
      codexStatusMsg.textContent = '正在安全寫入設定檔…';
      codexStatusMsg.style.color = '#2563eb';
    }
    try {
      const customCatalogPath = codexCatalogInput ? codexCatalogInput.value.trim() : "";
      const msg = await window.__TAURI__.core.invoke('codex_apply', {
        customPath: customPath || null,
        model: model || null,
        allowOverride: allowOverride,
        customCatalogPath: customCatalogPath || null
      });
      if (codexStatusMsg) {
        codexStatusMsg.textContent = msg;
        codexStatusMsg.style.color = '#10b981';
      }
      appendLog('SUCCESS', '套用 Codex 設定成功', msg);
      await checkCodexConfigStatus();
    } catch (err) {
      if (codexStatusMsg) {
        codexStatusMsg.textContent = `套用失敗: ${err}`;
        codexStatusMsg.style.color = '#ef4444';
      }
      appendLog('ERROR', '套用 Codex 設定失敗', err);
    }
  });
}

if (codexRollbackBtn) {
  codexRollbackBtn.addEventListener('click', async () => {
    const customPath = codexPathInput ? codexPathInput.value.trim() : "";
    if (codexStatusMsg) {
      codexStatusMsg.style.display = 'flex';
      codexStatusMsg.textContent = '正在還原先前設定…';
      codexStatusMsg.style.color = '#2563eb';
    }
    try {
      const msg = await window.__TAURI__.core.invoke('codex_rollback', {
        customPath: customPath || null
      });
      if (codexStatusMsg) {
        codexStatusMsg.textContent = msg;
        codexStatusMsg.style.color = '#10b981';
      }
      appendLog('SUCCESS', '還原 Codex 設定成功', msg);
      if (codexPreviewBox) codexPreviewBox.style.display = 'none';
      await checkCodexConfigStatus();
    } catch (err) {
      if (codexStatusMsg) {
        codexStatusMsg.textContent = `還原失敗: ${err}`;
        codexStatusMsg.style.color = '#ef4444';
      }
      appendLog('ERROR', '還原 Codex 設定失敗', err);
    }
  });
}

// 頁面載入完成後自動偵測 Codex 設定路徑
detectCodexPath();

const retainCheckbox = document.getElementById('retain-gateway');
if (retainCheckbox) {
  retainCheckbox.addEventListener('change', async () => {
    try {
      await window.__TAURI__.core.invoke('set_retain_on_exit', { retain: retainCheckbox.checked });
      appendLog('INFO', `視窗關閉策略已更新：${retainCheckbox.checked ? '保留背景閘道' : '隨視窗停止閘道'}`);
    } catch (_) {}
  });
}

const autostartCheckbox = document.getElementById('autostart-gateway');
const autostartStatus = document.getElementById('autostart-status');
if (autostartCheckbox) {
  // 查詢目前開機自啟設定狀態
  window.__TAURI__.core.invoke('get_autostart_gateway').then(enabled => {
    autostartCheckbox.checked = !!enabled;
    if (autostartStatus) {
      autostartStatus.textContent = enabled ? '🟢 已開啟（開機背景開閘）' : '⚪ 未啟用';
    }
  }).catch(() => {
    if (autostartStatus) autostartStatus.textContent = '';
  });

  autostartCheckbox.addEventListener('change', async () => {
    const previous = !autostartCheckbox.checked;
    try {
      const enabled = await window.__TAURI__.core.invoke('set_autostart_gateway', { enabled: autostartCheckbox.checked });
      autostartCheckbox.checked = !!enabled;
      if (autostartStatus) {
        autostartStatus.textContent = enabled ? '🟢 已開啟（開機背景開閘）' : '⚪ 未啟用';
      }
      appendLog('SUCCESS', `開機自動開閘設定已更新：${enabled ? '已啟用（開機自動在背景啟動閘道，不開 UI）' : '已關閉開機自動開閘'}`);
    } catch (err) {
      autostartCheckbox.checked = previous;
      appendLog('ERROR', '開機自動開閘設定失敗', err);
      if (autostartStatus) {
        autostartStatus.textContent = `⚠️ 設定失敗：${err}`;
      }
    }
  });
}

// === VS Code (Copilot & Agent) 整合模組 ===
const vscodeDeployBtn = document.getElementById('vscode-deploy-btn');
const vscodeDeployActionBtn = document.getElementById('vscode-deploy-action-btn');
const vscodeCheckStatusBtn = document.getElementById('vscode-check-status-btn');
const vscodePreviewBtn = document.getElementById('vscode-preview-btn');
const vscodePathInput = document.getElementById('vscode-path-input');
const vscodeDetectInfo = document.getElementById('vscode-detect-info');
const vscodeStatusMsg = document.getElementById('vscode-status-msg');
const vscodePreviewBox = document.getElementById('vscode-preview-box');
const vscodeDiffOutput = document.getElementById('vscode-diff-output');

async function checkVSCodeStatus() {
  const path = vscodePathInput ? vscodePathInput.value.trim() : '';
  try {
    const res = await window.__TAURI__.core.invoke('vscode_status', { customPath: path || null });
    if (vscodePathInput && !vscodePathInput.value) {
      vscodePathInput.value = res.target_path;
    }
    let desc = '檔案存在: ' + (res.exists ? '是' : '否');
    if (res.exists) {
      desc += ' | 配置模型: ' + res.model_count + ' 款';
      desc += ' | ModelBridge 管理: ' + (res.configured_by_bridge ? '是' : '否');
      if (res.has_backup) desc += ' (有備份檔可還原)';
    }
    if (vscodeDetectInfo) vscodeDetectInfo.textContent = desc;
    return res;
  } catch (err) {
    if (vscodeDetectInfo) vscodeDetectInfo.textContent = 'VS Code 狀態檢查失敗: ' + err;
  }
}

async function deployToVSCode() {
  const path = vscodePathInput ? vscodePathInput.value.trim() : '';
  if (vscodeStatusMsg) {
    vscodeStatusMsg.style.display = 'flex';
    vscodeStatusMsg.textContent = '🚀 正在部署模型配置至 VS Code…';
    vscodeStatusMsg.style.color = '#2563eb';
  }
  appendLog('INFO', '開始部署聚合模型至 VS Code chatLanguageModels.json…');
  try {
    const res = await window.__TAURI__.core.invoke('vscode_deploy', { customPath: path || null });
    if (vscodeStatusMsg) {
      vscodeStatusMsg.style.display = 'flex';
      vscodeStatusMsg.textContent = '✅ ' + res.message + ' [路徑: ' + res.target_path + ']';
      vscodeStatusMsg.style.color = '#10b981';
    }
    appendLog('SUCCESS', 'VS Code 部署成功: ' + res.message, res);
    await checkVSCodeStatus();
  } catch (err) {
    if (vscodeStatusMsg) {
      vscodeStatusMsg.style.display = 'flex';
      vscodeStatusMsg.textContent = '❌ 部署失敗: ' + err;
      vscodeStatusMsg.style.color = '#ef4444';
    }
    appendLog('ERROR', 'VS Code 部署失敗: ' + err);
  }
}

if (vscodeDeployBtn) {
  vscodeDeployBtn.addEventListener('click', async () => {
    vscodeDeployBtn.disabled = true;
    const old = vscodeDeployBtn.textContent;
    vscodeDeployBtn.textContent = '⏳ 正在部署…';
    try {
      await deployToVSCode();
    } finally {
      vscodeDeployBtn.disabled = false;
      vscodeDeployBtn.textContent = old;
    }
  });
}

if (vscodeDeployActionBtn) {
  vscodeDeployActionBtn.addEventListener('click', deployToVSCode);
}

if (vscodeCheckStatusBtn) {
  vscodeCheckStatusBtn.addEventListener('click', checkVSCodeStatus);
}

if (vscodePreviewBtn) {
  vscodePreviewBtn.addEventListener('click', async () => {
    try {
      const preview = await window.__TAURI__.core.invoke('vscode_preview', { customPath: vscodePathInput ? vscodePathInput.value.trim() || null : null });
      if (vscodePreviewBox && vscodeDiffOutput) {
        vscodePreviewBox.style.display = 'block';
        vscodeDiffOutput.textContent = preview;
      }
      if (vscodeStatusMsg) {
        vscodeStatusMsg.style.display = 'flex';
        vscodeStatusMsg.textContent = '預覽完成：已產生 VS Code 配置 JSON（金鑰已遮蔽保護）';
        vscodeStatusMsg.style.color = '#10b981';
      }
    } catch (err) {
      if (vscodeStatusMsg) {
        vscodeStatusMsg.style.display = 'flex';
        vscodeStatusMsg.textContent = '預覽失敗: ' + err;
        vscodeStatusMsg.style.color = '#ef4444';
      }
    }
  });
}

// 初始化偵測 VS Code
checkVSCodeStatus();
