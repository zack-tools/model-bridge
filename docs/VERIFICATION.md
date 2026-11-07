# 驗證紀錄

最後更新：2026-11-07（Asia/Taipei）

## macOS 原生驗收紀錄（2026-11-07）

- **作業系統與硬體**：macOS 27.2（26B123）、Apple Silicon（ARM64）。
- **Rust 工具鏈**：`cargo 1.99.1`（位於 `$HOME/.local/share/app-agg-001/cargo`）。
- **核心二進位**：CLIProxyAPI v8.0.57 (`c2f402b45643f0740d70b9010fd7d8d0aab7ef65`)，釘選 darwin/aarch64 SHA-256 完整核對通過。
- **測試結果**：
  - `agg-core` 單元測試：**42 passed, 0 failed, 1 ignored** (Windows x64 壓縮檔)。
  - `instance` 實例鎖與啟動保護測試：**3 passed, 0 failed**。
  - `lifecycle` 核心生命週期與崩潰測試：**5 passed, 0 failed**。
  - `protocol` 協定與 loopback 回呼測試：**13 passed, 0 failed, 1 ignored**。
  - 釘選核心隔離測試：`oauth_client_with_pinned_core_starts_and_cancels_without_login` 於 macOS ARM64 本機通過。
- **桌面殼建置**：`cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml` 成功，原生視窗成功開啟並正常運作（PID 49312，已整合本機持久登入、ChatGPT Desktop 模型清單目錄同步與 Codex 設定適配器）。
- **外部服務維持隔離**：使用者環境既有之 `CLIProxyAPI`（PID 1991，監聽 8317）維持獨立隔離，未受干擾。

## Gate 2 與 Gate 3 原生驗收狀態總表（macOS ARM64）

| 供應商 | 驗收閘門 | 狀態 | 客觀證據 | 限制與邊界 |
| --- | --- | --- | --- | --- |
| **ChatGPT / Codex** | **Gate 2: Loopback OAuth 登入** | **通過 (PASSED)** | 1. 僅監聽 `127.0.0.1:1455`<br>2. 瀏覽器回呼 `GET /codex/callback?code=...` 轉送側車 200 clean HTML，網址參數清除<br>3. 隔離目錄生成 `auth/codex-*-plus.json`<br>4. 介面歷經 `AwaitingConfirmation`，使用者點擊「確認本人完成」後轉為 `Authenticated` 且 `live=true` | 憑證目前存放於 0700 隔離暫存目錄；停止閘道或重置會刪除暫存憑證。未改用系統 Keyring。 |
| **ChatGPT / Codex** | **Gate 3: 串流與工具回合驗收 (文字/代碼)** | **通過 (PASSED)** | 使用者明確核取額度同意方塊。測試模型：<br>- `gpt-6-astra`<br>- `gpt-6.1-terra`<br>- `gpt-6-luna`<br>- `gpt-6.1-sol`<br>- `gpt-5.6-sol`<br>- `gpt-6-sol`<br>- `gpt-5.6-terra`<br>- `gpt-5.6-luna`<br>- `codex-auto-review`<br>所有模型兩回合串流工具呼叫皆回傳 200 OK，隨機 16-byte nonce challenge 比對精確吻合，`X-CPA-TRACE-ID` 16 位 trace index 與帳號憑證嚴格匹配（verified live） | 消耗真實帳號額度（已取得授權）。測試採最小 payload 探測以節省用量。 |
| **ChatGPT / Codex** | **Gate 3: 串流與工具回合驗收 (圖像生成)** | **不支援 (端點限制)** | 模型（共 8 款）：`gpt-image-1.5`, `gpt-image-2`, `gpt-image-2.5-flare`, `gpt-image-2.5-sunburst`, `gpt-image-2.5`, `gpt-image-2.6-flare`, `gpt-image-2.6-sunburst`, `gpt-image-2.6`。<br>發起串流工具呼叫時，上游端點立即回傳 HTTP 503 Service Unavailable，拒絕串流工具往返。閘道依規範捕捉並標示為不支援。 | 圖像生成模型端點本身不具備串流工具回合調用能力，非閘道轉發瑕疵。 |
| **Antigravity** | **Gate 2: Loopback OAuth 登入** | **通過 (PASSED)** | 1. 僅監聽 `127.0.0.1:51121`<br>2. 瀏覽器回呼 `GET /antigravity/callback?state=...&code=...` 轉送側車 200 clean HTML，網址參數清除<br>3. 隔離目錄生成 `auth/antigravity-*.json`，綁定 GCP 專案<br>4. 使用者點擊「確認本人完成」，狀態轉為 `Authenticated` 且 `live=true` | 憑證目前存放於 0700 隔離暫存目錄；停止閘道會清除。 |
| **Antigravity** | **Gate 3: 串流與工具回合驗收 (Gemini)** | **通過 (PASSED)** | 使用者明確核取額度同意方塊。測試模型（共 5 款）：<br>- `gemini-3.8-flash-high`<br>- `gemini-3-flash`<br>- `gemini-3.1-flash-image`<br>- `gemini-3.1-flash-lite`<br>- `gemini-3.5-flash-lite`<br>所有模型雙回合串流工具呼叫 200 OK（延遲 628ms~3.2s），第一回合帶出 `check` 與 `call_id`，第二回合隨機 nonce 核驗精確吻合，兩回合 `X-CPA-TRACE-ID` 嚴格匹配 `auth_index`（verified live） | 依規格書排他規則，Claude 系列模型自 Antigravity 模型發現清單中自動過濾，不作驗收。 |

## 分層狀態

| 層級 | 結果 | 說明 |
| --- | --- | --- |
| build-pass，核心函式庫 | pass | `cargo test --workspace` 通過 |
| build-pass，Tauri 桌面殼 | pass | macOS ARM64 原生建置二進位生成 |
| native-ui-pass，macOS 開窗 | pass | 原生視窗啟動並開窗運作中（PID 49312，附帶即時執行日誌與設定面板） |
| unit-pass | pass | 44 passed，涵蓋設定、HTTP、OAuth 狀態機、歸屬比對、串流證據解析、模型目錄同步與設定適配 |
| protocol-pass，mock | pass | 13 passed，涵蓋 loopback 回呼轉送、state 檢查、埠衝突、取消釋放 |
| protocol-pass，釘選核心 | pass | 釘選 sidecar 啟動、金鑰與未登入安全取消通過 |
| Gate 1：Loopback 回呼轉送器 | pass | 僅限 127.0.0.1 監聽（1455 與 51121），已驗證 state、埠釋放與 clean HTML |
| Gate 2：macOS 真人登入 | pass | 雙供應商（Codex 1455 / Antigravity 51121）由使用者完成系統瀏覽器登入，Loopback 安全回傳並清除網址參數，上游確認後本人確認通過，分別產生隔離憑證 |
| Gate 3：串流與工具回合驗收 | pass | 使用者明確同意額度消耗。雙供應商真實模型（Codex 文字/代碼模型、Antigravity Gemini 模型）完成雙回合串流工具呼叫，`X-CPA-TRACE-ID` 16 位憑證索引嚴格匹配，隨機 nonce challenge 核驗通過（verified live）；不支援串流工具之 `gpt-image` 依預期回傳 503 拒絕 |
| 憑證本機持久保存 | pass | 登入憑證存於應用本機目錄 `runtime/auth/`（0700，絕不上傳），停止閘道不再刪除憑證，重啟自動恢復 session |
| ChatGPT Desktop 模型目錄同步 | pass | 支援將聚合與驗證模型同步寫入 `model_catalog.json`（自動備份且保留原詳細屬性），供 ChatGPT Desktop UI 選用 |
| ChatGPT Desktop 目錄 Schema 修復與自動測試 | pass | 補齊 `supported_reasoning_levels`、`default_reasoning_level`、`support_verbosity`、`shell_type` 結構並支援繼承模板；`codex doctor` 驗證 `config.load` 通過；自動啟動 ChatGPT Desktop 驗證正常運作、無崩潰 |
| 圖像模型過濾（Image Models Exclusion） | pass | 在 `registry.rs`、`attribution.rs` 與 `codex.rs` 全面過濾含 `image` 之模型；現有 `model_catalog.json` 已清除 9 款 image 模型，`codex doctor` 維持 pass |
| Codex 設定預覽、套用與還原 | pass | 支援 `config.toml` 安全 diff 預覽、受控覆寫（自動備份）與還原（Rollback） |
| Windows 原生驗收 | 未驗證 | Windows ZIP 與二進位驗證代碼已具備，但尚未在 Windows 原生環境執行 |

## 關鍵安全防護檢驗

1. **Loopback 繫結保證**：
   - `LoopbackCallbackServer` 固定繫結 `127.0.0.1:1455`（Codex）與 `127.0.0.1:51121`（Antigravity），非本機請求一律拒絕。
   - 瀏覽器授權完成後，網址列參數透過 `history.replaceState` 即時清除，避免 Token / Code 留在網址列。
2. **三階段登入狀態分離**：
   - 瀏覽器接收回呼 (`200 OK`) ➔ 上游回報完成 (`AwaitingConfirmation`) ➔ 本人確認 (`Authenticated` 且 `live=true`)。
   - 任何一步驟未完成，模型發現與驗證皆拒絕執行。
3. **去識別化診斷**：
   - 修復「無法確定唯一可用帳號」之通用錯誤掩蓋，精確傳回 HTTP 狀態碼與憑證筆數，不輸出憑證內容、Token 或金鑰。
4. **Gate 3 證據鏈嚴格歸屬**：
   - 第一回合與第二回合標頭之 `X-CPA-TRACE-ID` 16 位憑證索引皆必須精確吻合所屬帳號之 `auth_index`。
   - 以 16-byte 高強度隨機 nonce 生成 Challenge，核對第二回合模型輸出。
   - 桌面 UI 強制要求勾選額度消耗同意方塊。
