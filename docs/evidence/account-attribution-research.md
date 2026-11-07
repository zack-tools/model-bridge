# 帳號歸屬：釘選上游程式碼調查

釘選版本：CLIProxyAPI v8.0.57，Git commit `c2f402b45643f0740d70b9010fd7d8d0aab7ef65`。2026-11-07 在 macOS 唯讀檢查該 commit 的公開原始碼，未使用真實帳號。

- [`internal/logging/cpa_trace.go`](https://github.com/router-for-me/CLIProxyAPI/blob/c2f402b45643f0740d70b9010fd7d8d0aab7ef65/internal/logging/cpa_trace.go) 定義 `X-CPA-TRACE-ID`，其格式包含選擇時間、憑證索引與請求 ID；標頭在回應標頭送出前加入。
- [`sdk/api/handlers/handlers.go`](https://github.com/router-for-me/CLIProxyAPI/blob/c2f402b45643f0740d70b9010fd7d8d0aab7ef65/sdk/api/handlers/handlers.go) 為非 WebSocket 請求設定所選憑證索引的追蹤回呼。
- [`handlers_stream_bootstrap_test.go`](https://github.com/router-for-me/CLIProxyAPI/blob/c2f402b45643f0740d70b9010fd7d8d0aab7ef65/sdk/api/handlers/handlers_stream_bootstrap_test.go) 的上游測試檢查串流執行所選憑證與追蹤索引；已在 macOS 執行該測試並通過，使用模擬執行器與憑證，不是實際帳號回合。
- [`auth_files.go`](https://github.com/router-for-me/CLIProxyAPI/blob/c2f402b45643f0740d70b9010fd7d8d0aab7ef65/internal/api/handlers/management/auth_files.go) 的管理資料包含 `auth_index` 欄位。

推論：在限定版本及無可信度降級的情況下，可研究將回應標頭中的索引與管理介面的帳號索引比對，作為帳號歸屬的一項證據。這不是強制指定帳號的方法，也不證明重試、標頭送出後的選擇變化、缺失標頭或所有供應商的行為；需要針對實際 `/v1/responses`、兩個供應商、重試及錯誤路徑做隔離測試，並完成真實回合後才能作驗收判定。不要記錄完整帳號資訊、憑證或原始追蹤標頭。

2026-11-07：在 macOS 以 Go 1.27.3 執行釘選來源的 `TestFormatCPATraceID`、`TestCPATraceIDMiddlewareRequiresAuthIndexBeforeResponseCommit` 與 `TestExecuteStreamWithAuthManager_SelectedAuthCallbackReceivesAuthID`，均通過。這些是上游隔離測試，不代表本產品整合或真實帳號驗收通過。
