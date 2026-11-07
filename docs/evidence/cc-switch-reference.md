# CC Switch 原始碼參考

檢查日期：2026-11-07。本機安裝版本顯示 4.0.26；本次原始碼參考為 [farion1231/cc-switch](https://github.com/farion1231/cc-switch) commit `66e4362b60cf0d93d9776cfe549c2b625e1c6b79`，兩者是否完全一致未確認。

- [macOS 退出事件處理](https://github.com/farion1231/cc-switch/blob/66e4362b60cf0d93d9776cfe549c2b625e1c6b79/src-tauri/src/lib.rs)：專案區分 `ExitRequested` 與 `Exit`，對系統退出提供清理流程。本專案採取自己的限時退出清理設計；強制終止仍可能造成孤兒程序。
- [Codex 設定管理](https://github.com/farion1231/cc-switch/blob/66e4362b60cf0d93d9776cfe549c2b625e1c6b79/src-tauri/src/codex_config.rs)：使用 TOML 資料結構、寫入保護及模型目錄處理。本專案仍須自行驗證設定路徑、並存衝突與回復，沒有直接覆寫 CC Switch 設定。
- [Codex 客戶端模型目錄](https://github.com/farion1231/cc-switch/blob/66e4362b60cf0d93d9776cfe549c2b625e1c6b79/src-tauri/src/services/provider/codex_client_catalog.rs)：上游實作關注客戶端啟動時的模型目錄快照與重啟狀態；這是待驗證的整合線索，不能直接認定本機 Codex 桌面必然相同行為。

既有 v1 範圍與發布閘門不變；不因參考而自動啟用 Claude、自動故障切換或複製 CC Switch 全部功能。
