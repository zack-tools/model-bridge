# 以後才做的真實登入

這份說明給使用者自己的機器。這次雲端環境沒有完成瀏覽器登入，也沒有送出會消耗額度的請求。

1. 在要驗的作業系統上，用 `scripts/fetch-core.sh` 的同一組校驗和檢查官方壓縮檔。macOS 與 Windows 檔名見 `vendor/cliproxyapi/checksums.txt`。
2. 先看條款提示。ChatGPT 帳號的 Codex OAuth 不是 OpenAI API 金鑰計費。
3. 只用上游已經實作的流程：管理 API 的 `oauth/auth-url`，或二進位的 `-codex-login`／`-antigravity-login`，並讓系統瀏覽器完成。不要改 state、PKCE 或回呼。
4. 若回應是帳號警告、停權或登入失敗，停掉該提供者。不要換一條未記載的身分 API。
5. 使用者在場時，才對選定模型做一次真實編碼或工具回合。做之前要明確同意會消耗該帳號額度。
6. 桌面 Codex 是否讀到同一份 `config.toml`、要不要重啟，必須在該 OS 的 GUI 上確認。CLI 成功不算數。
7. 記錄 OS、CPU、核心 v8.0.57 與壓縮檔 sha256。不要把 access token、refresh token、cookie 或授權碼寫進紀錄。

macOS Keychain、Windows Credential Manager／DPAPI、Linux Secret Service 都還沒接到這個 sidecar。在上游提供可替換的權杖儲存之前，不要宣稱權杖不會落地。
