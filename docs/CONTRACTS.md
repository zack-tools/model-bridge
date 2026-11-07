# 契約

時區 Asia/Taipei。核心釘選 CLIProxyAPI **v8.0.57**，git `c2f402b45643f0740d70b9010fd7d8d0aab7ef65`。下列路由來自該 commit 的 `internal/api/server_routes.go`、`internal/api/server_management_v8.go` 與管理文件，不是猜的。

## 路徑

Codex 客戶端 → 本機 `http://127.0.0.1:<port>/v1` Responses → sidecar → 使用者手動選定的帳號與模型。

| 用途 | 方法與路徑 |
| --- | --- |
| 健康 | `GET /healthz`、`HEAD /healthz`，200 且 `{"status":"ok"}` |
| 模型目錄 | `GET /v1/models`，需要客戶端金鑰 |
| Responses | `POST /v1/responses` |
| 管理 | `/v8/management`，本機也要管理金鑰 |
| 開始登入 | `GET /v8/management/oauth/auth-url?provider=codex` 或 `antigravity` |
| 輪詢 | `GET /v8/management/oauth/status?state=` |
| 取消 | `DELETE /v8/management/oauth/session?state=` |
| 瀏覽器回呼 | sidecar 自己的 `GET /codex/callback`、`GET /antigravity/callback` |

管理金鑰用 `Authorization: Bearer` 或 `X-Management-Key`。客戶端閘道金鑰在這次釘選二進位上，`Authorization: Bearer` 可以通過 `/v1/models`。Codex 文件另寫 `X-Api-Key`；mock 兩種都收，釘選核心這次只確認了 Bearer。

## 我們寫給 sidecar 的設定

只綁 `127.0.0.1`。`server.port` 不可為 0，因為上游會把它當成 8317。`management.allow-remote` 為 false。`server.discovery.enabled` 為 false。控制台與控制台自動更新關閉。`routing.session-affinity` 為 false，`routing.retry.request-retry` 為 0。這不是帳號輪替功能；上游若同時有多份憑證，仍可能在同一輪裡選擇，本 PoC 不新增輪替或 fallback。

v8.0.57 的 `AuthenticateManagementKey` 在 `secret-key` 與環境密鑰都空著時，會直接回 403 `remote management key not set`，不會先比對 `-password`。`internal/cmd/run.go` 在傳入 `-password` 時還會裝上 10 秒 keep-alive 關閉。因此監督器把管理金鑰寫進模式 `0600` 的產生設定，不傳 `-password`。金鑰是檔案，不是系統鑰匙圈。上游文件說明文 `secret-key` 會在載入時被 bcrypt；這次沒有把暫存檔留起來再讀一次雜湊。

OAuth 憑證由官方 sidecar 的 `NewFileTokenStore()` 寫入 `oauth.auth-dir` 的 JSON。說明書沒有把這個儲存接到 Keychain、Credential Manager 或 Secret Service 的旗標。`store_secret` 一律拒絕，也不另寫一份明文代替。

## Codex 設定

偵測只認兩種 CLI 候選：

- 有 `CODEX_HOME` 時：`$CODEX_HOME/config.toml`
- 否則：`$HOME/.codex/config.toml`（OpenAI config reference 寫的使用者設定位置）

ChatGPT Desktop／Codex GUI 的路徑與重啟條件，macOS、Windows、Linux 都還沒證明。偵測不會發明那些路徑，也不會建立家目錄。

套用只改呼叫者給的暫存檔。它會預覽 diff、同目錄備份、原子改名，並保留無關 TOML 與註解。`model_provider` 使用 `custom`，提供者名稱 `name` 為 `Custom`，`wire_api` 為 `responses`，`requires_openai_auth` 為 true，閘道金鑰放在 `experimental_bearer_token` 與 `http_headers.X-Api-Key`。同時透過 `sync_chatgpt_desktop_auth` 安全同步本地憑證，維持 ChatGPT Desktop 帳號登入態。預覽裡的金鑰會紅化，磁碟上的設定仍要有金鑰，Codex 才送得出去。

已有別的 `model_provider`、內文出現 `cc-switch`，或所有權雜湊對不上，就拒絕覆蓋。還原只在雜湊仍是我們寫入的那一版時進行。

## 模型與登入狀態

手動清單的來源只有 `chatgpt_codex` 與 `antigravity`。目錄觀察不新增可選列，也不能標成 verified。名稱或 `owned_by` 像 Claude／Anthropic 的會被擋下。`resolve` 只接受完全相同的 id 或別名。

登入狀態是 `loggedOut`、`authorizing`、`authenticated`、`expired`、`error`。沒有使用者在場的 success 會變成 error，不計成 live。mock 可以進入 `authenticated`，但 `live` 仍是 false。警告要先被看過並明確結束，才回到 `loggedOut`；不會自動重跑同一條流程。

日誌會去掉 access／refresh token、cookie、authorization、管理金鑰、`code` 與 `state` 查詢值。
