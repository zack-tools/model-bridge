const status = document.getElementById('status');
const start = document.getElementById('start');
const stop = document.getElementById('stop');
let generation = 0;
let discoveryGeneration = 0;

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
  } catch (_) {}
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

function resetAccounts() {
  ++discoveryGeneration;
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
    account.querySelector('p').textContent = '閘道已停止；本地保存之登入憑證完整保留，再次啟動可直接使用';
  }
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
    status.textContent = result.state === 'healthy'
      ? `閘道健康運作中：127.0.0.1:${result.port} (已自動套用至 ChatGPT Desktop Custom 提供者)`
      : `閘道無法回應健康檢查：127.0.0.1:${result.port}`;
    if (result.state === 'healthy') {
      if (pulseIndicator) pulseIndicator.className = 'pulse-dot running';
      if (quickStatusText) quickStatusText.textContent = `🟢 運作中 (埠號 ${result.port})`;
      if (metricPort) metricPort.textContent = `127.0.0.1:${result.port}`;
      if (metricStatusHint) metricStatusHint.textContent = '健康狀態良好 • 背景常駐中';
      stop.disabled = false;
      start.disabled = true;
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
    for (const account of document.querySelectorAll('.account')) {
      const provider = account.dataset.provider;
      try {
        const msg = await window.__TAURI__.core.invoke('oauth_action', { provider, action: 'poll' });
        account.querySelector('p').textContent = msg;
      } catch (_) {
        account.querySelector('p').textContent = '尚未登入';
      }
    }
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
      const changed = await window.__TAURI__.core.invoke('codex_suspend', { customPath });
      switchMessage = changed ? '；已切回原生供應商與預設模型清單' : '；未發現由 ModelBridge 管理的設定';
    } catch (error) {
      switchMessage = `；無法自動切回原生設定：${error}`;
      appendLog('ERROR', '閘道已停止，但設定切換失敗', error);
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

for (const account of document.querySelectorAll('.account')) {
  const result = account.querySelector('p');
  for (const button of account.querySelectorAll('button')) {
    button.addEventListener('click', async () => {
      const current = generation;
      const provider = account.dataset.provider;
      const action = button.dataset.action;
      for (const item of account.querySelectorAll('button')) item.disabled = true;
      result.textContent = '處理中…';
      appendLog('INFO', `帳號 [${provider}] 操作 [${action}] 開始`);
      try {
        const message = action === 'begin'
          ? await window.__TAURI__.core.invoke('oauth_start', { provider, terms: document.getElementById('terms').checked })
          : await window.__TAURI__.core.invoke('oauth_action', { provider, action });
        if (current === generation) result.textContent = message;
        appendLog('SUCCESS', `帳號 [${provider}] 操作 [${action}] 結果`, message);
      } catch (error) {
        if (current === generation) result.textContent = `操作未完成：${error}`;
        appendLog('ERROR', `帳號 [${provider}] 操作 [${action}] 失敗`, error);
      } finally {
        for (const item of account.querySelectorAll('button')) item.disabled = false;
      }
    });
  }
}

async function initializeGateway() {
  try {
    if (await window.__TAURI__.core.invoke('auto_start_enabled')) {
      const result = await window.__TAURI__.core.invoke('gateway_status');
      if (result.state === 'needs_recovery') {
        await window.__TAURI__.core.invoke('clear_run_guard');
      }
      if (result.state === 'stopped' || result.state === 'needs_recovery') {
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
  appendLog('INFO', `正在向閘道查詢 [${provider}] 之可用模型清單…`);
  try {
    const models = await window.__TAURI__.core.invoke('discovered_models', { provider });
    if (current !== generation || query !== discoveryGeneration) return;
    if (select && models.length > 0) {
      select.replaceChildren();
      for (const model of models) {
        const opt = document.createElement('option');
        opt.value = model;
        opt.textContent = model;
        select.appendChild(opt);
      }
      if (verifyBtn) verifyBtn.disabled = false;
    }
    for (const model of models) {
      const item = document.createElement('li');
      item.id = `model-entry-${model.replace(/[^a-zA-Z0-9_-]/g, '_')}`;
      const isGemini = model.startsWith('gemini');
      const badgeIcon = isGemini ? '✨' : '⚡️';
      item.textContent = `${badgeIcon} ${model}（可用）`;
      list.appendChild(item);
    }
    message.textContent = models.length ? `已發現 ${models.length} 個可用模型` : '目前沒有可顯示的模型';
    if (metricModels && models.length) {
      metricModels.textContent = `${models.length} 款模型`;
    }
    appendLog('SUCCESS', `查詢 [${provider}] 模型成功，發現 ${models.length} 個可用模型`, models);
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
      verifyStatus.textContent = `驗證成功：${result}`;
      appendLog('SUCCESS', `Gate 3 驗證通過：模型 [${model}]`, result);
      const entry = document.getElementById(`model-entry-${model.replace(/[^a-zA-Z0-9_-]/g, '_')}`);
      if (entry) {
        entry.textContent = `✓ ${model}（verified live）`;
        entry.style.fontWeight = 'bold';
        entry.style.color = '#10b981';
      }
      const codexModelInput = document.getElementById('codex-model-input');
      if (codexModelInput && !codexModelInput.value) {
        codexModelInput.value = model;
      }
    } catch (error) {
      verifyStatus.textContent = `驗證失敗：${error}`;
      appendLog('ERROR', `Gate 3 驗證失敗：模型 [${model}]`, error);
    } finally {
      verifyBtn.disabled = false;
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
      metricDesktopHint.textContent = res.current_model ? `預設: ${res.current_model}` : '保持登入 • 雙向同步中';
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
      codexStatusMsg.textContent = '🚀 [1/3] 正在同步可用模型清單至 model_catalog.json…';
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
        appendLog('SUCCESS', `[1/3] 模型目錄同步完成 (總計 ${summary.total_models} 款可用模型)`);
      } catch (catErr) {
        appendLog('WARN', `[1/3] 模型清單同步提示: ${catErr}`);
      }

      // 步驟 2: 套用 Custom 閘道端點至 config.toml
      if (codexStatusMsg) codexStatusMsg.textContent = '🚀 [2/3] 正在套用 Custom 閘道端點至 config.toml…';
      const applyMsg = await window.__TAURI__.core.invoke('codex_apply', {
        customPath: customPath || null,
        model: model || null,
        allowOverride: allowOverride
      });
      appendLog('SUCCESS', `[2/3] 套用閘道設定成功: ${applyMsg}`);

      // 步驟 3: 雙向同步登入憑證 (auth.json)
      if (codexStatusMsg) codexStatusMsg.textContent = '🚀 [3/3] 正在同步 ChatGPT Desktop 登入憑證 (保持登入)…';
      const authSummary = await window.__TAURI__.core.invoke('sync_chatgpt_auth');
      appendLog('SUCCESS', `[3/3] 登入憑證同步完成: ${authSummary.message}`);

      // 完成
      const totalModels = summary ? summary.total_models : 16;
      const doneMsg = `✅ 一鍵整合全部完成！已成功套用 Custom 供應商端點、同步 ${totalModels} 款模型目錄，並維持 ChatGPT Desktop 官方帳號登入態。`;
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
      codexStatusMsg.textContent = '正在安全雙向同步 ChatGPT Desktop 登入憑證…';
      codexStatusMsg.style.color = '#2563eb';
    }
    try {
      const summary = await window.__TAURI__.core.invoke('sync_chatgpt_auth');
      if (codexStatusMsg) {
        codexStatusMsg.textContent = summary.message;
        codexStatusMsg.style.color = '#10b981';
      }
      appendLog('SUCCESS', '同步 ChatGPT Desktop 登入憑證完成', summary);
    } catch (err) {
      if (codexStatusMsg) {
        codexStatusMsg.textContent = `憑證同步失敗: ${err}`;
        codexStatusMsg.style.color = '#ef4444';
      }
      appendLog('ERROR', '同步 ChatGPT Desktop 憑證失敗', err);
    }
  });
}

if (codexSyncCatalogBtn) {
  codexSyncCatalogBtn.addEventListener('click', async () => {
    const customCatalogPath = codexCatalogInput ? codexCatalogInput.value.trim() : "";
    if (codexStatusMsg) {
      codexStatusMsg.style.display = 'flex';
      codexStatusMsg.textContent = '正在同步可用模型清單至 ChatGPT Desktop (model_catalog.json)…';
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
      const preview = await window.__TAURI__.core.invoke('codex_preview', {
        customPath: customPath || null,
        model: model || null,
        allowOverride: allowOverride
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
      const msg = await window.__TAURI__.core.invoke('codex_apply', {
        customPath: customPath || null,
        model: model || null,
        allowOverride: allowOverride
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
