# APP-AGG-001 PHASE 1 PoC

未發行的私人草稿。這個目錄只證明一件事：薄的桌面殼可以和釘選的官方 CLIProxyAPI sidecar 用同一份契約溝通。它不是 MVP，也不是可安裝的正式版。

macOS 與 Windows 仍是產品要驗的平台。Linux 只是這次多做的評估，不構成發行閘門，也不代表所有發行版或 CPU 都支援。

## 這次有什麼

- `crates/agg-core`：共用契約。loopback 設定、模型清單、OAuth 狀態、Codex 設定預覽、行程監督都在這裡。
- `agg-poc`：無視窗的檢查程式。`doctor` 只報告這台機器看得到的事實。
- `apps/desktop`：Tauri 2.12.5 薄殼原始碼。此環境的 Rust 是 1.83，低於該 crate 要求的 1.90，而且沒有 WebKitGTK，所以沒有編譯、沒有開窗。
- `vendor/cliproxyapi`：v8.0.57 的授權、校驗和與釘選說明。二進位不進 git，用 `scripts/fetch-core.sh` 下載。

ChatGPT 訂閱不等於 OpenAI API 計費，也不表示任意模型都能呼叫。Claude 不進入手動清單。`/v1/models` 只算發現，不算驗證。

## 在這台機器上跑過的命令

```sh
cargo test --workspace
./scripts/fetch-core.sh
cargo test --test core_integration -- --ignored
cargo run -p agg-core --bin agg-poc -- doctor
```

結果寫在 `docs/VERIFICATION.md`。未跑的項目維持未驗證。

## 尚未驗證

- macOS、Windows 的編譯、原生視窗、系統匣或選單列
- 使用者在場的瀏覽器 OAuth，以及任何會消耗供應商額度的真實編碼回合
- ChatGPT Desktop／Codex GUI 的實際設定路徑與是否需要重啟
- 簽章、notarization、付費 CI

原生桌面要另外準備工具鏈，見 `docs/LINUX.md` 與 `docs/VERIFICATION.md`。不要把 Wine 或這次的 Linux headless 結果當成 Windows 或 macOS 通過。
