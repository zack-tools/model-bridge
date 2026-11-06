# 驗證紀錄

2026-11-06。工作目錄 `/workspace`。Rust `1.83.0`。沒有付費 CI，沒有改簽章或外部權限。

## 分層

| 層級 | 結果 |
| --- | --- |
| build-pass，Linux x86_64 headless 核心 | pass。`cargo test --workspace` 能編譯並跑完 |
| build-pass，Tauri 殼 | 未驗證。Rust 1.83 < Tauri 2.12.5 要求的 1.90，且 WebKitGTK 4.1 不在 |
| build-pass，macOS／Windows | 未驗證 |
| unit-pass | pass。lib 16、lifecycle 3、protocol 2，0 failed |
| protocol-pass，mock | pass。`tests/protocol.rs` |
| protocol-pass，釘選核心 | pass。見下方 JSON：`protocol_pinned_core` 為 `pass` |
| core-integration-pass，Linux x86_64 | pass。只代表這顆釘選的 linux/amd64 二進位 |
| oauth-live-pass | 未驗證。有啟動並取消，沒有瀏覽器，也沒有標成 live |
| native-ui-pass | 未驗證。macOS、Windows、Linux 桌面都沒開窗 |

## 命令

```text
cargo test --workspace
```

lib 16 passed；lifecycle 3 passed；protocol 2 passed；`pinned_linux_core` ignored。約 1 秒內結束（lifecycle 的 nohealth 案例約 0.91 秒）。

```text
./scripts/fetch-core.sh
```

印出 `.cache/cliproxyapi/CLIProxyAPI_8.0.57_linux_amd64.tar.gz` 與 `cli-proxy-api`。壓縮檔 sha256 等於釘選值。內層二進位 sha256 為 `047e384525c93abe1c98cdf3769ed5b720f621030f9777254c2985c11904c985`。`-h` 印出 `CLIProxyAPI Version: 8.0.57, Commit: c2f402b4`。

```text
cargo test --test core_integration -- --ignored --nocapture --test-threads=1
```

`pinned_linux_core` passed，3.11 秒。報告寫入 `/opt/cursor/artifacts/phase1-core-check.json`。

```text
cargo run -p agg-core --bin agg-poc -- doctor
```

Ubuntu 24.04.5 LTS；`linux/x86_64`；session unset；DISPLAY 有設定；Wayland 未設定；webkitgtk-4.1、gtk3、ayatana 都是 absent；Secret Service 為 `headless-absent-no-session-bus`；決定為 `refuse-no-quiet-plaintext-fallback`。

跑完後 `~/.cli-proxy-api` 與 `~/.codex` 仍不存在。沒有改使用者的 Codex 設定。

## 釘選核心報告摘要

- 健康 `200`，`/proc/net/tcp` 看到 127.0.0.1 監聽，沒有 0.0.0.0 或 IPv6 wildcard
- 對非 loopback 位址的額外連線探測是 `null`（沒有可用結果），所以不能用它證明拒絕外網；loopback 證據來自 `/proc/net/tcp`
- 不帶管理金鑰的 `GET /v8/management/config` 是 `401`
- 帶金鑰時讀到 `host=127.0.0.1`、`allow-remote=false`
- `/v1/models` 用 Bearer 得到 200。可選清單維持 0。沒有 live verified。這次回應裡符合 Claude／Anthropic 過濾的筆數是 0；這只描述該次回應，不是目錄永遠沒有其他名字
- Codex OAuth：HTTP 200，主機 `auth.openai.com`，接著取消，狀態回到 LoggedOut，`live=false`，auth 目錄沒有 token 欄位
- Antigravity OAuth：HTTP 200，主機 `accounts.google.com`，同樣取消且沒有憑證材料
- `user_present_marks=0`

手動另開一次二進位時，stdout 有上游自己的週期工作：Antigravity 版本每 3 小時重新整理，並取到 `2.29.0`；Grok CLI 版本也啟動了週期重新整理。監督器結束時會殺掉行程。本 PoC 沒有再寫一個輪詢器。控制台自動更新在產生的設定裡是關的；上述版本重新整理仍會在上游行程裡啟動。

## 仍要做的最小驗證

1. 在 macOS 與 Windows 各自用對應的釘選壓縮檔核對 sha256，編譯 Tauri 殼（Rust ≥ 1.90 與該 OS 的 WebView），打開視窗。
2. 使用者在場時完成兩個瀏覽器 OAuth，並各做一次會消耗額度的編碼回合。
3. 在每個 OS 上確認桌面 Codex 讀哪一份設定、要不要重啟。CLI 路徑不算。
4. Linux 若要繼續評估桌面，在 Ubuntu 24.04 圖形工作階段裝 WebKitGTK 4.1 後開窗，並分開記錄 X11 與 Wayland 的系統匣。這仍不是產品閘門。
