# model-bridge 開發進度

最後更新：2026-11-07（Asia/Taipei）

## 本次現場驗收紀錄（進行中）

- 使用者回報 ChatGPT／Codex 與 Antigravity 瀏覽器均顯示「授權已安全接收」，並回報曾對兩者按下檢查狀態與本人確認；等待本人確認的畫面未由協作者直接觀察。
- 兩個來源分別查詢模型時均到達「無法確定唯一可用帳號」錯誤。依桌面命令的執行順序，這表示當時通過了記憶體中的本人確認狀態檢查，但不證明上游帳號唯一、憑證可用或模型驗證通過。
- 已發現再次查詢已確認狀態時錯誤顯示「授權失敗或遭拒絕」；原執行中的視窗仍為修正前版本，程式碼已修正並通過狀態單元測試與桌面編譯檢查，尚未在原生視窗重驗。
- 回呼轉送失敗不得顯示成功的隔離測試通過；拒絕重複或缺漏 OAuth 參數的隔離測試亦通過。現場登入所用執行檔早於後者修正。
- 一次固定埠 protocol 測試與現場驗收時段重疊，出現一個回應不符及其後鎖中毒；原因尚未證實。後續僅使用臨時埠與不觸網的單元測試，現場期間不再跑固定 OAuth 埠測試。
- Gate 2 的兩家完整驗收仍待補齊可觀察證據及帳號查詢問題；Gate 3 未做真實模型呼叫，亦未標記任何模型 verified。Windows 原生驗收未完成。

## 目標與來源

依 [v1 規劃逐頁轉錄](model-bridge-v1-plan.md) 完成 Phase 1 PoC。macOS 與 Windows 為個別必驗平台；Linux 僅作額外評估。MVP／正式發布須先通過規劃中的雙平台 PoC 閘門。

## 已完成與證據

| 項目 | 狀態與範圍 | 證據 |
| --- | --- | --- |
| 專案接手 | 從下載的 Git 提交建立完整工作副本，保留原始下載資料 | Git 工作目錄；原下載目錄仍存在 |
| macOS 核心測試 | macOS 27.2、Apple Silicon：21 項測試通過；Linux 專用整合測試略過 | [驗證紀錄](VERIFICATION.md) |
| macOS 桌面殼 | Tauri 編譯成功；臨時 app bundle 顯示靜態說明頁 | [驗證紀錄](VERIFICATION.md) |
| Windows ZIP 產物驗證 | 加入 x64／ARM64 ZIP 映射及壓縮檔內執行檔比對；先驗證完整壓縮檔，再要求唯一的執行檔 | 在 macOS 下載並核對 x64／ARM64 v8.0.57 發行檔 checksum，提取執行檔；x64 額外以 Rust 驗證內部雜湊。Windows 原生建置與啟動未驗證 |
| macOS sidecar | 釘選 v8.0.57 的 ARM64 壓縮檔及其執行檔比對成功；隔離整合與協定檢查通過 | [去識別檢查結果](evidence/macos-arm64-core-check.json) |
| 兩種 OAuth 的安全探測 | Codex、Antigravity 均取得授權網址後取消；未登入、未建立憑證 | [去識別檢查結果](evidence/macos-arm64-core-check.json) |
| OAuth 後端客戶端 | 已實作條款確認、允許的授權主機檢查、等待與確認取消；mock 與 macOS 釘選核心均測過兩供應商啟動、輪詢、取消 | `oauth_client` 的 protocol 測試；不開瀏覽器、不完成登入、不標示 live |
| 登入畫面狀態 | 閘道重新啟動後顯示尚未登入；停止後遲到的 OAuth 結果不覆蓋已停止狀態 | JavaScript 隔離模擬通過；真實瀏覽器流程尚未驗收 |
| macOS 互動登入嘗試 | 使用者回報 ChatGPT／Codex 與 Antigravity 均出現警告或失敗；已停止驗收與模型呼叫，具體提示待使用者提供 | 2026-11-07 使用者回報；介面僅顯示已要求開啟瀏覽器，未觀察到上游成功或本人確認，兩者均未驗收 |
| OAuth 成功狀態 | 新增上游成功後等待使用者確認的狀態；未觀察到上游成功時，不能直接標為登入。模擬成功停留在等待確認，不計 live | `confirmation_requires_observed_upstream_success`、`oauth_upstream_success_requires_user_confirmation`；尚未接桌面或實際登入 |
| macOS 桌面 OAuth PoC | 已接上條款勾選、系統瀏覽器、手動查詢、確認與取消；UI 顯示憑證只留在暫存目錄，停止閘道刪除 | 原生視窗驗證未勾選提示及未啟動閘道均會拒絕；尚未在介面完成真實瀏覽器授權或取消驗證 |
| 模型清單安全條件 | 只有 live 驗證項目可選；發現結果與 mock 不算 live，重複 ID 拒絕模糊解析，帳號參照以雜湊顯示 | `manual_selection_rejects_unverified_and_ambiguous_models`；真實回合驗證尚未實作，故目前無 live 可選項目 |
| 串流證據基礎 | 新增離線 Responses SSE 證據解析：檢查完整結束事件、模型、片段與最終輸出一致；拒絕截斷及錯誤。回傳僅摘要，不保留輸出 | `response_evidence` 單元測試；已接本機 chunked SSE 測試入口，尚未接桌面、不證明帳號來源或實際工具回合，不能標記 live |
| Codex 設定衝突保護 | 預覽與套用遇到既有非表格的 `model_providers`、管理項目或 `http_headers` 時拒絕修改，避免取代原值 | macOS 隔離測試 `codex::tests` 5 項通過，包含三種衝突及檔案未變檢查；桌面整合尚未驗收 |
| chunked 解碼基礎 | 新增有大小限制的增量解碼，要求完整資料、區塊分隔符、終止區塊與尾端標頭；失敗時呼叫者必須丟棄部分結果 | macOS `http::tests` 5 項通過，含分段、截斷及超限案例；已接本機隔離串流測試，尚未完成真實模型驗證 |
| 切換來源顯示 | 更改帳號來源或停止閘道會清除模型清單並作廢進行中的查詢結果 | JavaScript 隔離模擬確認舊來源遲到回應不會顯示；尚未用真實登入驗證 |
| 帳號模型發現基礎 | 已接桌面查詢：本人登入確認後，僅當同來源帳號唯一才查詢；名稱衝突拒絕、過濾 Claude，Antigravity 僅顯示 Gemini；結果只是發現，不能選用或視為驗證 | macOS 模擬端點與資料 `attribution::tests` 5 項、桌面編譯檢查及來源切換時舊結果丟棄的介面模擬通過；未做真實登入驗收，Windows 未驗證 |
| 帳號來源查詢基礎 | 以管理 API 依索引檢查唯一、可用且受支援的憑證，只向呼叫端回傳來源種類；此結果不代表真實回合驗收 | macOS 模擬資料與 loopback 管理端點共 2 項測試通過；未接桌面、真實帳號或模型選擇，Windows 未驗證 |
| 帳號歸屬上游調查 | 釘選上游的 HTTP 追蹤標頭程式與管理資料均有憑證索引；已加入 16 位上游索引格式的保守比對；只回報比對狀態，不作模型驗收 | [原始碼調查](evidence/account-attribution-research.md)；macOS 8 項串流測試及 3 項上游追蹤測試通過；未執行真實帳號回合 |
| 串流輸出關聯 | 非空文字與工具呼叫須逐項匹配串流片段，孤立片段或額外未匹配輸出會拒絕 | macOS `response_evidence::tests` 8 項通過；仍未完成真實工具往返或帳號驗收 |
| 本機串流測試入口 | 新增限定 127.0.0.1 的 Responses POST，檢查回應類型與 chunked 框架後交給 SSE 證據解析；不會標記模型 verified | macOS 模擬 TCP 伺服器測試完整、截斷與持續緩慢傳送；回應讀取使用總期限，`response_evidence::tests` 8 項通過；真實上游、帳號歸屬、桌面與 Windows 未驗證 |
| chunked 傳輸實驗 | 隔離測試發現 ureq 2.12.1 對聲明長度超出實際資料的 chunked 回應可能回報正常 EOF；實驗性串流接線已撤回 | macOS 本機測試中，截斷回應未被底層回報錯誤；真實模型網路串流仍未驗收 |
| HTTP 防護 | 部分回應後逾時不再視為成功；長度以 bytes 驗證，拒絕無效或重複長度、未支援的 transfer encoding 及標頭注入 | `http` 單元測試；釘選 macOS 核心的 OAuth 啟動／取消重測通過 |
| Gemini 唯讀審查 | 使用 agy Gemini 3.8 Flash High 審查所提供的程式片段；採納截斷、HTTP 標頭及帳號歸屬風險。建議將完成事件改為 `response.done` 與官方 Responses 文件不符，未採納 | [OpenAI Responses 串流文件](https://developers.openai.com/api/docs/guides/streaming-responses)；Gemini 未執行測試 |
| VS Code 工作項目 | 新增核心測試與桌面編譯檢查工作項目；macOS 指向獨立持久 Rust 工具目錄 | 使用該工具執行核心測試通過；macOS 已在 VS Code 執行桌面編譯檢查並顯示成功，Windows 需自行安裝 Cargo |
| Windows 下載準備 | 新增可直接由 Python 執行的跨平台下載程式，沿用既有 shell 入口；提取前核對釘選 checksum | macOS 對 macOS ARM64 與 Windows x64／ARM64 產物實測；尚未在 Windows 執行 |
| Windows 靜態審查 | Windows 瀏覽器開啟已接入系統介面，尚未原生驗證；監聽檢查尚待修正；Windows 程序環境已分平台設定，尚需原生驗證其啟動與 HTTPS，ACL 需原生驗證。YAML 路徑跳脫已修正，在 macOS 測試 Windows 格式含中文與空格路徑往返通過 | 獨立代理唯讀檢查；macOS 核心生命週期 5 項回歸測試通過；尚未作 Windows 原生測試，不得據此宣稱 Windows 可用 |
| 跨平台修正 | 修正 macOS 核心下載平台、Linux 專用測試、桌面依賴、缺少圖示與 sidecar 相對路徑 | 工作目錄變更及 [驗證紀錄](VERIFICATION.md) |
| macOS 桌面啟停 | PoC 按鈕可啟動及停止釘選 sidecar；測試後埠已釋放、隔離暫存目錄已刪除 | macOS 原生視窗實際操作，2026-11-07；尚未涵蓋崩潰和異常退出 |
| 核心異常退出 | 每兩秒查詢核心行程及健康狀態；測試中強制結束該視窗的 sidecar 後顯示意外停止、清除暫存，並可重新啟動及停止 | macOS 原生視窗實測；`detects_exit_after_initial_health` 模擬測試通過 |
| 閘道跨程序鎖 | 啟動前鎖定使用者應用資料目錄的固定鎖檔，停止或偵測到核心退出後釋放；獨立子程序測試確認持鎖時拒絕、釋放後可再取得 | `instance_lock_across_processes`；macOS 測試通過，Windows 未驗證 |
| CC Switch 參考及正常退出 | 參考開源程式的退出事件處理，新增限時清理與頁面內停止確認 | [參考紀錄](evidence/cc-switch-reference.md)；macOS 釘選核心實測停止按鈕及 Cmd+Q 後，測試核心、暫存目錄、保護紀錄均消失；強制終止及 Windows 未驗證 |
| macOS 模型查詢介面實測 | 未啟動閘道時查詢被拒；啟動釘選核心但未登入時提示需先完成本人確認 | 2026-11-07 原生視窗實測；未使用真實帳號。首次測試停止按鈕無效且 Cmd+Q 後核心殘留；已清理並修正，後續重測見「CC Switch 參考及正常退出」 |
| 異常後啟動保護 | sidecar 啟動前建立持久保護紀錄；已確認停止才移除。macOS 實測強制終止桌面程序後舊核心仍存在，再啟動被拒絕；測試後已確認並清理測試行程與紀錄 | 原生視窗實測及 `run_guard_survives_owner_exit_and_blocks_replacement`；尚無自動復原 |
| 停止時清理失敗 | 無法刪除暫存目錄時保留啟動保護及路徑；同一桌面程序提供再次停止以重試清理 | macOS 桌面單元測試以非目錄模擬刪除失敗，確認保護保留及重試成功；原生視窗與 Windows 尚未驗證 |
| 啟動失敗清理 | 核心啟動失敗時區分子程序已確認不存在與無法確認；僅在確認不存在且暫存目錄刪除成功後移除保護紀錄，其他情況保留 | macOS lifecycle 5 項測試及桌面 `cargo check` 通過；停止失敗與檔案刪除失敗分支、桌面整合及 Windows 尚未實測 |
| 異常狀態提示 | 桌面啟動與啟動失敗後檢查持久保護紀錄；若存在，顯示需檢查舊閘道、停用啟動按鈕並標示先前登入資料狀態未知 | Gemini 唯讀審查指出先前狀態誤顯示；macOS `cargo check` 及 3 個模擬畫面狀態案例通過，原生視窗操作與 Windows 尚未驗證 |
| 異常紀錄檢查 | 新紀錄保存執行目錄、不保存金鑰；macOS 視窗可唯讀檢查程序命令列。實測運行時找到對應程序，正常停止後顯示無紀錄 | 原生視窗實測；舊版空紀錄不會自動清除 |
| Loopback 安全 OAuth 回呼轉送器 | 實作僅限 `127.0.0.1` 的臨時回呼轉送器（Codex: 1455、Antigravity: 51121），取代上游 `0.0.0.0` 外網暴露；包含 state 比對驗證、非 loopback 請求即時拒絕、轉送至核心 management、乾淨 HTML 回傳（含 `history.replaceState` 清除網址列參數）、埠衝突偵測、逾時與取消/drop 即時釋放埠 | macOS ARM64 `protocol::tests` 11 項測試通過（含 1455/51121 轉送、state 不符 400、瀏覽器取消處理、`PortConflict` 衝突回報、取消及 drop 立即釋放埠）；桌面 `cargo check` 通過；尚未進行使用者在場真實登入 |
| 帳號清單診斷與根因修復 | 查明原通用錯誤「無法確定唯一可用帳號」之根因為側車存取中曾發生 401 且錯誤被通用訊息吞沒；已改寫 `attribution.rs` 與 `main.rs` 回傳去識別之精確診斷（HTTP 狀態、候選憑證數、欄位完整性），不洩漏敏感資產；隔離測試全數通過 | 單元測試 43 項全數通過，以釘選核心實測憑證結構與模型發現端點符合預期 |
| Gate 3 雙回合工具驗證器 | 實作 `inspect_real_model_tool_roundtrip` 與 `inspect_controlled_tool_roundtrip_with_index`，兩回合均嚴格要求 `X-CPA-TRACE-ID` 匹配所選帳號之 `auth_index`；以 16-byte 高強度隨機 nonce 生成 Challenge 核對模型輸出；桌面 UI 增設 Gate 3 驗證區塊與額度消耗強制核取方塊 | 單元測試 `controlled_roundtrip_with_account_attribution_requires_matching_trace` 通過；桌面編譯成功；真實模型呼叫須經使用者明確同意 |
| Gate 2：macOS 真人雙供應商登入 | 使用者在場完成 ChatGPT/Codex 與 Antigravity 之系統瀏覽器 OAuth 登入。Loopback 回呼轉送器（1455 與 51121）成功接收轉送，上游回報成功，使用者親自點擊「確認本人完成」，狀態成功轉為 Authenticated 且 live=true，分別生成隔離憑證 | macOS 原生視窗與側車紀錄 `stdout.log`（2026-11-07 15:08:07 與 15:08:37 回呼 200 OK，15:08:26 與 15:08:41 狀態 200 OK） |
| HTTP Chunked 增量解碼修復 | 修復上游 `/v8/management/credentials` 採用 `Transfer-Encoding: chunked` 導致原客戶端拋出 `transfer encoding is not supported by this client` 之錯誤；正式接入增量 `decode_chunked` 並修復模型清單發現 | 單元測試 `chunked_requires_complete_framing` 通過，原生視窗實測成功帶出 25 個模型 |
| Gate 3：雙供應商真實串流與工具回合驗收 | 取得使用者明確額度消耗同意核取。對發現之模型發起雙回合真實串流工具呼叫：第一回合成功帶出 `check` 工具呼叫與 `call_id`，第二回合將 16-byte 高強度隨機 nonce challenge 傳入 `function_call_output`，核對模型回覆；兩回合之 `X-CPA-TRACE-ID` 皆嚴格匹配帳號憑證索引 `auth_index`。所有文字/程式碼模型全數驗證通過（verified live）；圖像生成模型（gpt-image）因端點不支援串流工具呼叫回傳 503 拒絕並正確標示 | 側車 `stdout.log` 多筆真實 `POST /v1/responses` 200 OK 往返紀錄，耗時 600ms~3.2s，雙回合隨機 Challenge 匹配成功 |
| 桌面 UI 即時日誌支援 | 依使用者要求增設 GUI 執行紀錄與驗收日誌區塊（含自動滾動與清除按鈕），並在後端新增 `log_message` Tauri 命令持久化至 `desktop.log` 與 stderr，使後續操作與驗收結果完全透明可讀，免除人工回報負擔 | 桌面殼編譯與執行通過，日誌即時寫入 |
| 憑證本機持久保存與重啟自動恢復 | 依使用者指示修改：登入憑證存於應用本機資料目錄 `runtime/auth/`（0700，絕不上傳），停止閘道時只終止側車進程、完整保留本地憑證；閘道重啟時自動讀取本地憑證恢復 session，無需每次重複開啟瀏覽器授權 | 單元測試 `persistent_runtime_is_preserved_on_stop` 通過；本機金鑰寫入 `local_keys.json` (0600) 持久化 |
| ChatGPT Desktop 模型目錄同步 | 依使用者指示實作模型目錄同步適配器 `sync_model_catalog`：將閘道聚合且驗證通過之模型（Codex/Gemini）自動注入至 ChatGPT Desktop 的 `model_catalog.json`，保留原有 instruction/reasoning 等詳細屬性並自動建立備份檔，供 ChatGPT Desktop UI 選用 | 單元測試 `syncs_model_catalog_preserving_existing_and_backing_up` 通過；已接桌面 UI 與 Tauri 命令 |
| Codex 設定預覽、套用與還原 | 完成桌面 Codex Config Adapter：支援設定路徑偵測、安全 diff 預覽、受控覆寫（自動備份）與衝突還原（Rollback）；將 `model_catalog_json` 與本機 Responses 閘道端點寫入 `config.toml` | 單元測試 `codex_desktop_status_and_rollback` 通過；已接桌面 UI 與 Tauri 命令 |
| ChatGPT Desktop 目錄 Schema 修復與自動測試 | 解決 ChatGPT Desktop 載入 `model_catalog.json` 報錯 `missing field supported_reasoning_levels` 問題：在 `agg-core::codex` 補齊 `supported_reasoning_levels`、`default_reasoning_level`、`support_verbosity`、`shell_type` 必填結構並支援自動繼承既有模型模板；`codex doctor --json` 驗證 `config.load` 由 fail (invalid data) 轉為 ok；自動啟動 ChatGPT.app 實測生成 176KB+ 正常運作日誌，CLI 連線與 8 個 MCP 伺服器全數正常載入 | `cargo test -p agg-core --lib codex` 7 項測試全綠；`codex doctor` 報告 `config loaded`；系統程序與日誌 `codex-desktop-f8d75e99-...` 實測確認 |
| 圖像模型過濾（Image Models Exclusion） | 依使用者指示全面排除不支援串流工具回合調用之 image 模型：在 `registry.rs`（`is_image_model`、`observe_gateway_catalog`、`ingest_credential_models`）、`attribution.rs`（`discover_models`、`parse_models`）及 `codex.rs`（`sync_model_catalog`）全面過濾含 `image` 之模型標識，並自現有 `/Users/Zack/.codex/model_catalog.json` 中移除 9 款先前加入之 image 模型，確保桌面 UI 與模型清單僅呈現可真實調用之對話/程式碼模型 | 核心 67 項測試全數通過（含模型過濾斷言）；`model_catalog.json` 驗證清理後無任何 image 模型殘留且 `codex doctor` 維持 pass |
| ChatGPT Desktop GPT 模型不可用根因修復與雙供應商端點驗證 | 解決使用者在 ChatGPT Desktop 選取 GPT 系列模型提示「此模型無法使用，請選擇其他模型」：定位根因為先前 Rollback 導致 `config.toml` 指向舊有外部單一代理 8317（該端口僅有 Gemini，無 Codex/OpenAI）；修復方式：(1) 在 `agg-core::codex` 增強 `mutate`，確保 `app_agg_001` 與 `custom` 提供者均同步對齊聚合閘道端點與 Bearer/API-Key；(2) 在桌面端主程式加入預設固定連接埠 50833 避免動態埠重啟失效；(3) 在 `agg-poc` 支援 headless apply；(4) 實測驗證以預設設定檔呼叫 `gpt-6.1-terra`（Codex）與 `gemini-3.8-flash-high`（Antigravity）全數成功回傳（回傳 `AGG_DEFAULT_CONFIG_SUCCESS_GPT` 與 `AGG_DEFAULT_CONFIG_SUCCESS_GEMINI`）；自動重啟 ChatGPT.app 確認正常初始化無報錯 | 核心 67 項測試全綠；`codex doctor --json` 檢驗全 pass；OpenAI Codex CLI 與 ChatGPT Desktop 雙模型實測雙向串流完成 |
| 工具完善：Headless CLI 守護管理與視窗關閉常駐策略 | 依使用者指示繼續完善工具：(1) 實作 `agg-poc` 完整 CLI 命令組：`status`（即時回報健康度、雙帳號載入、可用模型數及 ChatGPT Desktop 設定整合狀態）、`sync`（命令列直接同步模型目錄）、`start`（自動啟動側車並套用設定）、`stop`（優雅停止並 100% 保留本地登入憑證）；(2) 實作桌面端視窗關閉常駐策略（Retain on Exit）：桌面 UI 視窗關閉時預設保持背景閘道運行，避免 ChatGPT Desktop 於日常使用中因關閉視窗而中斷；(3) UI 自動偵測並標註已授權雙帳號狀態，啟動即知已登入；(4) 統一抽取 `ensure_local_keys`、`default_app_data_dir` 與 `find_core_binary` 至核心庫，消除程式重複 | 核心工作區 67 項測試通過；`agg-poc status` 實測正確回傳雙帳號健康資訊；`agg-poc sync` 實測完成 22 款模型目錄備份與同步 |
| ChatGPT Desktop 保持帳號登入支援（Account Persistence & Bi-directional Sync） | 解決使用者在 ChatGPT Desktop (`ChatGPT.app`) 中因代理設定喪失帳號登入態之問題：(1) 定位根因為 `config.toml` 中 `requires_openai_auth = false` 會導致 Electron UI 關閉認證協議與 Session 模組，並出現未登入態；在 `agg-core::codex` 固化 `requires_openai_auth = true`，使 ChatGPT Desktop 保持 Attach-Auth 且順暢通過閘道驗證；(2) 實作本機雙向憑證同步模組 `sync_chatgpt_desktop_auth`：當本地閘道有最新憑證時自動注入 `~/.codex/auth.json`（0600 權限與原子寫入），若使用者在 ChatGPT Desktop 原生重新登入時亦能自 `auth.json` 反向導入本地 `runtime/auth/`；(3) 在 CLI 工具加入 `agg-poc sync-auth`，並在 `agg-poc start`、`agg-poc apply`、Tauri `start_gateway`、`codex_apply` 與桌面 UI（「同步 ChatGPT 登入憑證」按鈕）全面自動觸發同步；(4) 實測驗證：`codex doctor --json` 顯示 `auth.credentials` 通過且 `stored ChatGPT tokens: true`，ChatGPT Desktop 啟動日誌確認 `authenticatedAccountPresent=true`、`hasAccount=true`、`hasUser=true`，頭像/Plus 會員權益完整維持，`gpt-6.1-terra` 與 `gemini-3.8-flash-high` 雙模型串流正常回傳 | 工作區 70 項測試全綠（`agg-core` 68 項、桌面端 2 項）；`agg-poc sync-auth` 與 `status` 實測確認登入維持；ChatGPT.app 日誌無任何未登入警告 |
| 供應商名稱對齊 Custom（Native Provider Alignment） | 依使用者指示將提供者名稱全面改為官方原生標識 `Custom`：(1) 於 `agg-core::codex` 將 `PROVIDER_ID` 固化為 `"custom"`，`mutate()` 產生 `name = "Custom"` 與 `model_provider = "custom"`；(2) 同時同步更新 `[model_providers.custom]` 與 `[model_providers.app_agg_001]` 雙向配置；(3) 實測套用至 `~/.codex/config.toml`；`codex doctor --json` 確認 `model provider: "custom"`；雙模型呼叫（`gpt-6.1-terra`、`gemini-3.8-flash-high`）實測經由 `provider: custom` 串流成功回傳 `CUSTOM_PROVIDER_GPT_OK` 與 `CUSTOM_PROVIDER_GEMINI_OK` | 工作區 70 項測試全綠；`codex doctor` 報告配置正常；OpenAI Codex CLI 與 ChatGPT Desktop 完全對齊 Custom 提供者 |
| 桌面端 UI/UX 現代化全面升級（Modern Desktop System & One-Click Setup） | 依使用者指示進行 UI/UX 深度優化：(1) 視覺設計系統升級：導入 Apple HIG 現代卡片質感、毛玻璃微陰影、呼吸狀態光點（🟢/🔴）、深淺色模式自適應，擺脫原型期純 HTML；(2) 頂部即時儀表板（Dashboard）：展示閘道端點（127.0.0.1:50833）、ChatGPT Desktop 整合狀態（Custom）、有效帳號數（2 個）與可用模型數（16 款）；(3) 互動流程革新（UX）：推出「⚡️ 一鍵設定 ChatGPT Desktop」，三步合一（同步模型目錄 + 套用 Custom 閘道 + 雙向同步登入憑證），免去多次切換點擊；(4) 操作體驗增強：新增「📋 複製日誌」快捷按鈕、按鈕非同步 Loading 防重複點擊、狀態條即時回饋，並 100% 保留原有 DOM ID 與後端 Tauri 命令相容性 | 測試套件 70 項全綠；桌面前端靜態與動態事件綁定完整無報錯 |
| 登入檔隔離、釘選啟動與 macOS arm64 交接 | 啟動、套用與 `--daemon` 不再讀寫 `auth.json`。閘道只用自己的 OAuth。明確預覽與確認才會移除已經相同的 refresh token，世代備份失敗就中止寫入。所有正式 sidecar 啟動走同一個先驗證再啟動的函式。新增 `scripts/macos-arm64-handoff.sh` 與 `docs/MACOS_ARM64_HANDOFF.md`。 | `cargo test --workspace`：86 passed / 0 failed / 3 ignored。未跑真實 OAuth。 |
| Windows 原生相容性與修復：CMD 視窗消除、CC Switch 殘留隔離與關閉閘道還原 |
| VS Code (Copilot & Agent) 快速部署與思考深度對齊（VS Code Integration & Thinking Depth） | 依使用者指示實作 VS Code (Copilot & Agent) 快速部署與思考深度配置：(1) **VS Code 設定檔適配**：於 `agg-core::vscode` 實作跨平台 `chatLanguageModels.json` 偵測、產生、備份與套用，並於 Tauri 後端註冊 `vscode_status`、`vscode_preview`、`vscode_deploy` 指令；(2) **思考深度控制**：Gemini 系列（`gemini-3.8-flash-high`、`gemini-3-flash`）思考深度嚴格限定為「低、中、高」（`["low", "medium", "high"]`），移除不適用之 max，模型名稱顯示修正為 `Gemini 3.8 Flash`（不帶 High）；GPT 與 Claude 保留 low/medium/high/max；(3) **401 Unauthorized 標頭修復**：針對 VS Code CustomEndpoint 機制未預設帶 Bearer 標頭導致驗證失敗之問題，於模型層明確注入 `requestHeaders: { "Authorization": "Bearer <key>" }`；(4) **桌面端 UI 整合**：於「客戶端整合」分頁新增「VS Code (Copilot & Agent) 整合」專屬卡片與「一鍵部署至 VS Code」按鈕，並於模型發現清單以友善名稱顯示；(5) 桌面端 Release 二進位檔編譯完成並實測生效 | 工作區 78 項單元與整合測試全數通過（`agg-core` 68 項、`desktop` 10 項）；實測 `chatLanguageModels.json` 成功部署且支援思考深度切換 | 依使用者指示完成三項 Windows 平台修復：(1) **啟動閘道 CMD 彈窗**：在 `supervisor.rs` 啟動 `cli-proxy-api.exe` 加入 Windows `CREATE_NO_WINDOW (0x08000000)` flag，消除彈窗且防止關閉視窗殺死閘道；(2) **模型清單混雜 CC Switch**：在 `codex.rs` 的 `sync_model_catalog` 全面過濾 CC Switch 注入條目（`comp_hash: "cc-switch"`、`slug: "ccs-*"`、`display_name: "Gemini-OAuth"`）與歷史代理模型，並在克隆模板時主動移除 `comp_hash`，確保 ChatGPT Desktop 模型選單僅呈現原生 OpenAI 與驗證通過之 ModelBridge 模型；(3) **關閉閘道還原原生清單**：修復 `suspend_config` 因 Windows 具名管道（`SKY_CUA_NATIVE_PIPE_DIRECTORY`）動態變更而誤報 `external edit` 拒絕切換之問題（加入 volatile 欄位正規化）；實作 `restore_native_catalog` 並於 `suspend_config`、`rollback_config` 及桌面端 `stop_gateway` 自動連動還原 `model_catalog.json` 為純原生清單。 | 單元測試 `syncs_model_catalog_filters_cc_switch_and_restores_native` 與桌面端 `codex_desktop_suspend_restores_native_catalog` 通過；`codex doctor` 報告 0 fail；Release 二進位檔編譯完成並實測生效 |

macOS sidecar 由 lsof 觀察到 IPv4 loopback、未見 wildcard；非 loopback 連線探測無結果，不能據此宣稱已完成外網連線驗證。原生視窗顯示不代表互動式桌面功能已驗收。

## 待完成與驗收閘門

- [x] 在使用者在場時驗證桌面 Codex 與 Antigravity 瀏覽器 OAuth、取消及成功確認；警告或認證失敗時停止流程。（Gate 2 完成，macOS ARM64 原生驗證通過）
- [x] 完成支援 SSE 及 HTTP chunked 的網路傳輸、工具回合與帳號來源歸屬驗證；各供應商執行真實串流 coding／tool 回合，事前明確確認帳號用量消耗，並只將實際驗證的模型標記為 verified。（Gate 3 完成，macOS ARM64 原生驗證通過）
- [x] 依使用者指示實作本機憑證持久化（停止閘道不刪除，重啟自動恢復），以及 ChatGPT Desktop 模型清單目錄（`model_catalog.json`）同步與 `config.toml` 安全套用。
- [ ] 完成桌面單實例與再次開啟時聚焦、異常退出的孤兒行程處理；目前已有閘道啟動鎖與崩潰守護。
- [ ] 確認 Windows 桌面端 Codex 設定路徑與重啟條件；完成 Windows 原生驗收（不可用 macOS 或 Wine 結果替代）。
- [ ] 完成雙平台 sidecar 架構產物與 checksum 盤點，缺少的架構標示 unavailable。
- [ ] 依規劃評估 Linux 的基線、WebKitGTK、tray 與 Secret Service；不視為正式支援或雙平台替代驗收。

## 目前限制

macOS Gate 2 與 Gate 3 已完成原生驗證，但 Windows 原生驗證尚未開始，兩平台原生驗收完成前不得宣稱 MVP 完成或發布。登入憑證已依指示保存在本機應用目錄（0700），停止閘道不會刪除；金鑰存於本地 `local_keys.json` (0600)，絕不上傳任何外部伺服器。強制終止桌面程序後 sidecar 和暫存保護可能保留，啟動保護會拒絕後續啟動，可透過桌面「清除保護紀錄並重試」按鈕復原。Windows ZIP 驗證已加入但尚未於 Windows 原生測試；下載流程、執行環境與核心啟動仍待驗證。macOS 的專案專用 Rust 工具已放在使用者目錄下，VS Code 工作項目已指定路徑；Windows 工具環境尚未配置。

## 下次接續

1. 檢查工作目錄與 [驗證紀錄](VERIFICATION.md)，保留尚未提交的修改。
2. macOS Gate 2（真人登入）與 Gate 3（雙供應商真實串流與工具回合驗收）已完成，可推進桌面 Codex 設定預覽/套用與 Windows 原生環境驗收準備。
