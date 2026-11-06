# Linux 評估

Linux 是額外評估目標。它不取代 macOS 與 Windows 的原生驗收，也不承諾所有發行版或架構。

## 這台機器實際看到的

| 項目 | 觀察 |
| --- | --- |
| 發行版 | Ubuntu 24.04.5 LTS |
| 架構 | x86_64 |
| glibc | 2.39 |
| 工作階段 | `XDG_SESSION_TYPE` 未設定 |
| Wayland | `WAYLAND_DISPLAY` 未設定 |
| DISPLAY | 程序環境有設定；沒有開窗 |
| WebKitGTK 4.1 | `pkg-config` 報告 absent |
| GTK 3 | absent |
| Ayatana AppIndicator | absent |
| Secret Service | 沒有 session bus，也沒有 `secret-tool`。決定是 `refuse-no-quiet-plaintext-fallback` |

`agg-poc doctor` 會重印這些探測。探測失敗不會改去寫明文權杖。

## 擬議基線，不是支援承諾

若以後要在 Linux 上編譯 Tauri 視窗，建議先用 **Ubuntu 24.04 LTS x86_64**。理由只是 Tauri 2 的 Debian 說明使用 WebKitGTK 4.1，而這次觀察到的系統正是 Ubuntu 24.04。其他發行版要各自再驗。

Tauri 文件（2026-08-20 那一版前提頁）給 Debian 的套件是：

`libwebkit2gtk-4.1-dev`、`build-essential`、`curl`、`wget`、`file`、`libxdo-dev`、`libssl-dev`、`libayatana-appindicator3-dev`、`librsvg2-dev`

這次沒有安裝它們。`libayatana-appindicator3-dev` 出現在 Tauri 的一般 Linux 說明裡；本殼沒有啟用 `tray-icon` feature，避免為了未驗證的系統匣去加依賴。

Tauri 系統匣文件（2026-04-20）寫明：Linux 上即使圖示畫出來，滑鼠 enter／move／leave／click 事件仍不送出，右鍵選單仍可能出現。Wayland 與 X11 還要看桌面有沒有 StatusNotifier watcher。這兩種工作階段都還沒在真實桌面上操作過。

## 建置目標對照

| 目標 | 角色 | 這次 |
| --- | --- | --- |
| Windows x64 / arm64 | 產品擬議 | 未跑。有官方壓縮檔與校驗和 |
| macOS arm64 / amd64 | 產品擬議 | 未跑。有官方壓縮檔與校驗和 |
| Linux x86_64 | 額外評估 | headless 編譯、單元、釘選核心有跑。不是原生 UI |
| Linux aarch64 | 額外評估 | 官方壓縮檔在校驗和清單裡。這台機器不是該架構 |

CLIProxyAPI v8.0.57 的發行說明寫預設 Linux 包以 GLIBC 2.17 為基線並含動態外掛。這次用的是 `CLIProxyAPI_8.0.57_linux_amd64.tar.gz`，不是 no-plugin 包。解出的 ELF 標頭寫 `for GNU/Linux 2.6.32`，那是二進位註記；實際載入的是這台機器的 glibc 2.39。沒有在舊 glibc 上重跑。

最低作業系統版本仍未定。Tauri 前提頁寫 macOS Catalina 10.15 與 Windows 7，那是工具箱說明，不是本產品已測支援。

## 桌面殼

`apps/desktop/src-tauri` 釘 `tauri = 2.12.5`（crates.io checksum `5fb0244332cfb9cee7e2a568bf5da29a36ed8fdbd717f6f47a66b6b7b358c072`，`rust-version` 1.90）與 `tauri-build = 2.7.4`（checksum `adddb0e648752aaac5da9c3c3c6a68e1e87cab55e3b757b18a9441ef8c3ac614`）。此環境是 Rust 1.83.0，所以沒有 fetch、沒有 `cargo check` 這個殼。headless 核心不連結 WebKit。

在一台 Rust ≥ 1.90 且已裝上述套件的 Ubuntu 24.04 上，視窗建置會是：

```sh
cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml
APP_AGG_OPEN_UI=1 cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml
```

`devUrl` 指到 `127.0.0.1:18431`，這次沒有啟動那個位址。沒有系統匣實作。開窗成功仍只算 Linux 桌面評估，不算 macOS 或 Windows。
