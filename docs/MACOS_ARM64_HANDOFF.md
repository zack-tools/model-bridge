# macOS arm64 驗收

這份清單給一台 Apple Silicon Mac。雲端環境沒有跑真實 OAuth，也沒有碰 `~/.codex`、`~/.cli-proxy-api` 或 `~/Library/Application Support/io.github.zacktools.modelbridge`。

## 工具鏈

| 項目 | 要求 |
| --- | --- |
| 機器 | `uname -s` 為 Darwin，`uname -m` 為 arm64 |
| 工作區 `rust-version` | **1.88**（`Cargo.toml` 的 `[workspace.package]`；`zopfli 0.8.4` 需要這個版本） |
| 桌面殼 | **rustc ≥ 1.90**。`apps/desktop/src-tauri/Cargo.toml` 寫 `rust-version = "1.90"`，因為 Tauri 2.12.5 自己的 `rust-version` 是 1.90 |
| 建議安裝 | `rustup toolchain install stable` 後 `rustup default stable`，再 `rustc --version` 確認 ≥ 1.90 |
| Python | `python3`，給 `scripts/fetch-core.py` 與隔離路徑檢查 |

`prepare` 在 rustc 低於 1.88 或低於 1.90 時都會停。1.88 只能編 headless crate；開視窗一定要 1.90 以上。

## 隔離

`scripts/macos-arm64-handoff.sh` 每個子命令都會把下面四個目錄放在工作副本的 `.handoff/`，並在隔離沒有生效時退出：

| 變數 | 位置 |
| --- | --- |
| `HOME` | `.handoff/home` |
| `CODEX_HOME` | `.handoff/codex-home` |
| `APP_AGG_DATA_DIR` | `.handoff/app-data` |
| `APP_AGG_CORE_DIR` | `.handoff/core` |

另外設定 `APP_AGG_REQUIRE_ISOLATION=1`。桌面程式與 `agg-poc` 在這個旗標底下如果路徑跑出 `.handoff/`，直接退出。不要用 `run-desktop.sh` 做這次驗收；它沿用呼叫者的 `HOME`。

不會寫入：

- `~/.codex`
- `~/.cli-proxy-api`
- `~/Library/Application Support/io.github.zacktools.modelbridge`

閘道的 OAuth 權杖只留在 `.handoff/app-data/runtime/auth`。預設的啟動、套用與 `--daemon` 不讀寫 `auth.json`。登入隔離要先預覽再確認，而且不會把 refresh token 複製到另一個客戶端。

## 離線 rollback selftest

`scripts/macos-rollback-selftest.sh` 用 bash 載入這支交接腳本的隔離檢查，在 `.handoff/` 底下的暫存 `CODEX_HOME` 做兩次套用。它不啟動閘道，也不做 OAuth。`CODEX_HOME` 若解析到 `.handoff` 外面，腳本直接退出。從任何工作目錄都可以呼叫：

```sh
bash scripts/macos-rollback-selftest.sh
```

第一例追加 `[projects."/tmp/x"] trust_level = "trusted"` 再還原，`config.toml` 必須留下且只含那張表。第二例沒有外來表，還原後檔案必須消失。每例印 `PASS` 或 `FAIL`，任一失敗則退出碼非 0。不要用 zsh source 交接腳本。

## 命令

在倉庫根目錄：

```sh
rustup toolchain install stable
rustup default stable
rustc --version

./scripts/macos-arm64-handoff.sh prepare
./scripts/macos-arm64-handoff.sh core
./scripts/macos-arm64-handoff.sh build
./scripts/macos-arm64-handoff.sh isolate
./scripts/macos-arm64-handoff.sh window
```

視窗裡按「啟動閘道」。兩邊登入都要人在場，而且要明確接受檔案權杖儲存（CLIProxyAPI v8.0.57 使用 `NewFileTokenStore()`，沒有 Keychain 旗標）。沒有 `--accept-file-tokens` 時 `oauth-begin` 以狀態碼 1 結束，不會開始登入：

```sh
./scripts/macos-arm64-handoff.sh oauth codex --accept-file-tokens --open
./scripts/macos-arm64-handoff.sh oauth antigravity --accept-file-tokens --open
```

授權網址不會印出來。完成後在視窗按確認。然後對一個你剛登入、會消耗額度的模型做預覽、套用、一輪、還原。把 `<id>` 換成視窗裡實際列出的模型 id：

```sh
./scripts/macos-arm64-handoff.sh codex-preview --model <id>
./scripts/macos-arm64-handoff.sh codex-apply --model <id>
```

在 Codex 裡對該模型送一輪。不要把 access token、refresh token、cookie 或授權碼寫進紀錄。然後：

```sh
./scripts/macos-arm64-handoff.sh codex-rollback
./scripts/macos-arm64-handoff.sh stop
```

## 勾選

- [ ] `isolate` 印出的四個路徑都在工作副本的 `.handoff/` 裡
- [ ] `~/.codex`、`~/.cli-proxy-api`、`~/Library/Application Support/io.github.zacktools.modelbridge` 的修改時間沒有變
- [ ] 沒有 `--accept-file-tokens` 時 oauth 指令退出碼是 1
- [ ] Codex 與 Antigravity 都在隔離目錄完成登入
- [ ] 預覽 diff 沒有明文閘道金鑰
- [ ] 套用後 Codex 打到 `http://127.0.0.1:<port>/v1`，一輪有回應
- [ ] 還原後 `config.toml` 回到套用前
- [ ] 紀錄只有 OS、CPU、核心 v8.0.57 與壓縮檔 sha256，沒有權杖
