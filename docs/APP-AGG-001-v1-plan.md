## Page 1

2026/11/7 上午10:30APP-AGG-001 v1 聚合閘道架構與功能規劃
第1⾴（共11⾴）https://app-agg-001-v1-e35b.pages.grok.bot/
APP-AGG-001 v1ʛCodex 聚合閘道架構與功能
規劃
APP-AGG-001 v1 ⽅案已於 2026-11-06 12:13 Asia/Taipei 核准；Phase 1 PoC 已開始。
必驗平台：macOS＋Windows；Linux 為評估⽬標（非已確認正式⽀援）。程式開發已啟
動（私⼈未公開 Origin 草稿雲端開發，無公開發⾏）；OAuth／原⽣驗證與測試結果尚未
回報。時區 Asia/Taipei。
On this page
建議結論
⽬標與邊界
平台範圍
帳號與協議釐清
架構
技術棧基準
模組與資料契約
功能表
MVP 資訊架構
階段與驗證閘⾨
安全與合規
風險與 Go/No-Go
核准與信任改進
建議結論
核准狀態（2026-11-06）
12:13 Asia/Taipei：使⽤者同意⽅案，授權 APP-AGG-001 v1 與 Phase 1 PoC 開
⼯（非核准跳過閘⾨直接進正式／MVP 發⾏）。12:16：OS 另納入 Linux 評估。
程式⼯作已開始；測試、OAuth、原⽣桌⾯驗證尚未回報通過。開發為私⼈未公開
Origin 草稿雲端開發，無公開發⾏。
基準架構（已核准規劃）
採⽤「單⼀可⾒桌⾯ UX＋釘選官⽅ CLIProxyAPI ⼆進位 sidecar」。殼層⽤ Tauri
（與上游 EasyCLIProxyAPI 同族；該專案⽂件寫明 macOS menu bar 與
Windows system tray）。本機閘道只暴露 loopback 的 OpenAI Responses；登
入走上游已⽀援的系統瀏覽器 OAuth（Codex／ChatGPT 帳號、Antigravity）。
聚合清單只含 ChatGPT／Codex 與 Gemini／Antigravity 經驗證模型；不因

## Page 2

2026/11/7 上午10:30APP-AGG-001 v1 聚合閘道架構與功能規劃
第2⾴（共11⾴）https://app-agg-001-v1-e35b.pages.grok.bot/
聚合清單只含 ChatGPT／Codex 與 Gemini／Antigravity 經驗證模型；不因
Antigravity ⽂件列出 Claude 就對外暴露 Claude。
必驗平台已確認：macOS 與 Windows；MVP 兩邊都要過。Linux 僅為評估⽬標，不標為
已確認正式⽀援，亦不要求涵蓋所有發⾏版。架構變體與最低 OS／發⾏版版本屬擬議或評
估覆蓋，須⽤上游 core 產物與 runner 再確認。
「單⼀⼯具」指單⼀操作⾯與⽣命週期，不是單⼀ OS ⾏程。基準採殼＋sidecar 兩⾏程；
若改單⾏程，須另做 Go SDK embed PoC，通過前不可寫進 MVP 承諾。Wails 系統匣能
⼒本次未以官⽅⽂件核實，不列為基準。
任務 ID APP-AGG-001 v1
規劃狀態 approved · PoC in progress
時區 Asia/Taipei
平台（必驗） macOS + Windows
平台（評估） Linux（非正式⽀援承諾）
開發現況 程式已開⼯；測試／OAuth／native 驗證未回報；私⼈ Origin 草稿，無公開發⾏
⾸要未決 各必驗平台桌⾯ Codex 設定路徑／重啟條件；Linux 基線發⾏版／架構／依賴可
⾏性
⽬標與邊界
要做
1.單⼀輕量桌⾯⼯具，取代同時開
CC Switch 與 CLIProxyAPI
2.聚合模型清單：OpenAI／
ChatGPT（Codex OAuth）＋
Gemini（Antigravity OAuth）
3.本機穩定 Responses 閘道，供桌
⾯端 Codex 編碼選⽤
4.OAuth 必備；⼿動選模型；可套⽤
／還原 Codex 設定
5.macOS 與 Windows 皆納入 PoC
／MVP 必驗收
6.Linux 納入評估（可⾏性／
headless；不充 native 桌⾯證
明、不承諾全發⾏版）
不做（v1）
1.不新增未⽂件化的⾝分／登入流程
2.不因 Antigravity ⽀援 Claude 就暴
露 Claude
3.不做⾃動 failover／智慧路由為預
設
4.不重寫完整協議棧（優先複⽤上
游）
5.不複製 CC Switch 全功能（MCP
／Skills／⽤量⼤盤等）
6.不宣稱廠商官⽅背書本專案整合
7.不以 Wine／非原⽣環境代替
Windows／macOS 的 native 驗收
8.不以 Linux headless／可⾏性測試

## Page 3

2026/11/7 上午10:30APP-AGG-001 v1 聚合閘道架構與功能規劃
第3⾴（共11⾴）https://app-agg-001-v1-e35b.pages.grok.bot/
明、不承諾全發⾏版）
7.零額外花費依賴（不含既有帳號本
⾝⽤量；不擅⾃購買簽章憑證或註
冊開發者帳號）
8.不以 Linux headless／可⾏性測試
充 Windows／macOS native 桌⾯
通過
9.不把 Linux 標成已確認正式⽀援或
要求⽀援所有 distro
使⽤者以桌⾯客⼾端存取 Codex（稱 ChatGPT desktop）。本規劃不假定特定 App 版
本、設定路徑或相容性已驗證；每個平台的設定套⽤都可單獨被相容性結果卡住。
平台範圍
必驗平台（已確認）
macOS 與 Windows 為正式⽀援與 native 驗收必要條件。架構（CPU）與最低系
統版本尚未確認。
Linux（評估，2026-11-06 12:16）
使⽤者要求 OS 也考慮 Linux。本⾴將 Linux 列為評估⽬標：盤點基線發⾏版、最
低版本、架構、依賴與可⾏性。非已確認正式⽀援；不要求涵蓋所有 distro；
Linux 可⾏性／headless 測試不得當作 Windows／macOS native 桌⾯通過證
明。
建置／評估⽬標 狀態 說明
Windows x64 必驗·擬議覆蓋 須確認 CLIProxyAPI core 產物與 runner
Windows ARM64 必驗·擬議覆蓋 無產物則標 unavailable，不假裝⽀援
macOS Apple Silicon 必驗·擬議覆蓋 須確認 core 產物與 runner
macOS Intel 必驗·擬議覆蓋 無產物則標 unavailable
Linux（基線 distro／版本／架
構待評估）
評估中 共⽤邏輯可複⽤；發⾏版與套件矩陣未
定；非正式⽀援承諾
Tauri 殼與各 OS ⾏為（規劃事實）
主題 Windows macOS Linux（評估）
WebView WebView2（系統／ WKWebView（系統 WebKitGTK 與系統依

## Page 4

2026/11/7 上午10:30APP-AGG-001 v1 聚合閘道架構與功能規劃
第4⾴（共11⾴）https://app-agg-001-v1-e35b.pages.grok.bot/
WebView WebView2（系統／
可再發⾏；建置檢查
項）
WKWebView（系統
WebKit）
WebKitGTK 與系統依
賴；可⽤性未驗證
常駐入⼝ system tray
（EasyCLIProxyAPI
已寫）
menu bar
（EasyCLIProxyAPI
已寫）
tray 於 Wayland／
X11；⾏為與套件未驗
證
單實例 Named mutex／等效
鎖 + ⼆次啟動聚焦
等效單實例鎖 + ⼆次
啟動聚焦
等效鎖策略待評估
Sidecar 包裝 釘選 core；啟動前
checksum
同上；沙盒／公证另
列
打包格式／依賴檢查
未驗證
OAuth 系統瀏覽器；
localhost callback／
上游流程
同上 有桌⾯⼯作階段時同
策略；headless 另
案，不可靜默略過
設定家⽬錄 偵測 HOME／
PROFILE；不硬編碼
偵測 $HOME；不硬編
碼
偵測 $HOME／
XDG；不硬編碼
簽章／分發 選項；不擅⾃購證 選項；不擅⾃註冊
Apple Developer
發⾏通道未定；不承
諾全 distro 套件
簽章與 notarization 影響安裝摩擦與 Gatekeeper／SmartScreen 提⽰，但不構成「必須
先付費才能開 PoC」的隱性核准。未取得簽章時，⽂件須寫明未簽章安裝步驟與風險；取
得簽章屬另⼀次明確核准與成本決策。
帳號與協議釐清
概念 本規劃定義 注意
ChatGPT 帳號 →
Codex OAuth
透過上游⽀援的 Codex／
ChatGPT 登入取得本機可⽤憑
證，服務 Codex 編碼
≠ ⼀般 ChatGPT／OpenAI API
key 計費路徑；權益取決於帳號
本⾝⽅案
Gemini via Antigravity
OAuth
透過上游已列的 Antigravity 帳
號登入取得 Gemini 通道能⼒
上游 README／
EasyCLIProxyAPI 列有
Antigravity OAuth；≠ Google
官⽅背書本⼯具
Responses 閘道 本機對 Codex 暴露 OpenAI
Responses 相容入⼝（Codex
wire_api 僅 responses）
/models 清單 ≠ 實際可呼叫；
需分 discovered／verified／
unavailable
聚合清單 同⼀本機入⼝下，⼿動選擇 別名映射到實際 model id；預

## Page 5

2026/11/7 上午10:30APP-AGG-001 v1 聚合閘道架構與功能規劃
第5⾴（共11⾴）https://app-agg-001-v1-e35b.pages.grok.bot/
聚合清單 同⼀本機入⼝下，⼿動選擇
ChatGPT 或 Gemini 來源模型
別名映射到實際 model id；預
設不⾃動切換
合規提⽰
使⽤訂閱／帳號於非官⽅客⼾端或本機代理，受各服務條款與公告約束（以當下⽂
本為準）。產品必須在登入前顯⽰「請⾃⾏閱讀並遵守上游與服務商條款／公
告」；若偵測到認證失敗或帳號／登入警告，應停⽌該流程並提⽰使⽤者，不得規
避。
架構
請求路徑：桌⾯ Codex → 本機 loopback Responses → core 依⼿動選定帳號／模型轉
發。控制路徑：UX → services 管理 OAuth、模型驗證、Codex 設定擁有權與⾏程監督。
架構⽀援與「某 CPU 產物是否⿑」分開驗證。
桌⾯ Codex 客⼾端
本⼯具桌⾯ UX
macOS menu bar／Windows tray + 主視窗
App Services
Account · ModelRegistry · ConfigAdapter ·
ProcessSupervisor
CLIProxyAPI core
釘選版 sidecar（基準）或 SDK embed（PoC
後選）
ChatGPT／Codex OAuth 上游
 Antigravity OAuth 上游
loopback Responses
指令
啟動／checksum／健康檢查
偵測路徑／備份／原⼦寫入／還原
OAuth／API OAuth／API

## Page 6

2026/11/7 上午10:30APP-AGG-001 v1 聚合閘道架構與功能規劃
第6⾴（共11⾴）https://app-agg-001-v1-e35b.pages.grok.bot/
架構⽀援與「某 CPU 產物是否⿑」分開驗證。
技術棧基準
基準優點
直接使⽤上游 Responses／OAuth／
Antigravity 實作
core 版本可釘選、可回滾，checksum
可驗證
⼀個操作⾯同時管 core ⽣命週期與
Codex 設定
與 EasyCLIProxyAPI 已證明的雙平台
tray／menu bar 模式同族
基準代價
可⾒⼀ App ≠ 單 process
須維護雙平台 sidecar 產物矩陣
SDK 單⾏程路徑另需 PoC，不能寫進
MVP 承諾
未簽章安裝在兩 OS 都有額外使⽤者步
驟
授權與依賴
1.CC Switch、CLIProxyAPI 皆為 MIT：可整合，須保留著作權與許可⽂字
2.釘選 CLIProxyAPI 發⾏版（每架構 checksum）；禁⽌浮動 latest 當預設
3.依賴審計：殼層 crates／前端依賴、core ⼆進位來源、更新通道皆可追溯
4.不引入需付費雲端編譯代理；不擅⾃購買 Windows／Apple 簽章相關費⽤
模組與資料契約
模組 責任 輸入／輸出（概念）
選項 單⼀ UX⾏程 OAuth／tray
Tauri 殼 + 釘選 CLIProxyAPI sidecar是 基準 2（殼＋core） EasyCLIProxyAPI 已⽂件化 tray
／menu bar；OAuth 複⽤上游
Go embed SDK + 原⽣薄殼 是 ⽬標 1 需雙平台 PoC：OAuth
callback、憑證儲存、串流
Wails + Go 未定 視實作 本次未核實官⽅ tray ⽂件；不當
作基準
全⾃研協議轉換 是 1 偏離上游安全更新；超出 v1

## Page 7

2026/11/7 上午10:30APP-AGG-001 v1 聚合閘道架構與功能規劃
第7⾴（共11⾴）https://app-agg-001-v1-e35b.pages.grok.bot/
ProcessSupervisor 雙平台單實例鎖、啟動／停⽌／
崩潰重啟、checksum、健康檢
查
命令 → core 狀態（stopped／
starting／healthy／
degraded）
AccountService 啟動上游⽀援的系統瀏覽器
OAuth；撤銷／登出；條款提⽰
provider=chatgpt_codexʛ
antigravity → AccountRef（無
密⽂ token 欄）
ModelRegistry 發現、驗證、別名；狀態三分 ModelEntry：alias、
actualId、sourceAccount、
status、lastVerifiedAt、
capabilities
GatewayFacade loopback 位址、本機閘道
auth、/responses 可⽤性
baseUrl、localAuthMode、
listenPolicy
CodexConfigAdapter 執⾏期偵測設定路徑；備份、原
⼦寫入、外部修改、安全 diff
還原
DetectedPathʛApplyPlan／
RollbackPlan；保留無關鍵
SecretStoreAdapter
（條件）
上游儲存介⾯ PoC 通過後：
Windows Credential Manager
／DPAPI、macOS Keychain；
Linux 評估 Secret Service（有
則⽤、無／headless 須明⽰失
敗或安全降級，禁⽌靜默明⽂）
未通過前：沿⽤上游機制並標⽰
磁碟儲存事實，不承諾「密鑰永
不落地」
AuditLog 非敏感事件：登入開始／成功／
失敗類別、套⽤設定、驗證結
果、平台標籤
禁⽌寫入 token、refresh、
cookie、密碼
ModelEntry 狀態
discovered · 未驗證
來⾃ /models 或上游列舉，尚未實測
verified · 可⽤
已在該 OS 完成本機串流／⼯具回合抽樣
unavailable · 不可⽤
驗證失敗或上游拒絕；保留 lastVerifiedAt 與錯誤類別
capabilities 只記已觀察⾏為（streaming、tool calls、multi-turn、structured output
等）；缺測標 unknown。預設⼿動選模；不做⾃動 fallback。

## Page 8

2026/11/7 上午10:30APP-AGG-001 v1 聚合閘道架構與功能規劃
第8⾴（共11⾴）https://app-agg-001-v1-e35b.pages.grok.bot/
等）；缺測標 unknown。預設⼿動選模；不做⾃動 fallback。
建議⽬錄（僅規劃）
1.desktop/ — Tauri 殼；Windows tray／macOS menu bar 入⼝
2.services/ — 上述模組介⾯與實作
3.vendor/cliproxyapi/ — 各架構釘選 core、checksum 清單、授權檔
4.docs/ — 雙平台安裝、未簽章說明、驗收矩陣
5.不在倉庫或 Page 放置任何密鑰、token、帳號檔欄位範例值
功能表
功能 MVP說明
雙平台單實例啟動／停⽌ core 必備 崩潰可重啟；退出策略可選保留或停⽌閘
道
ChatGPT／Codex OAuth 必備 系統瀏覽器；上游⽀援流程
Antigravity OAuth 必備 系統瀏覽器；上游⽀援流程
聚合模型清單（兩來源） 必備 ⼿動選擇；別名→實際 id
模型驗證狀態 必備 discovered／verified／unavailable（含
平台標籤）
本機 Responses 閘道 必備 loopback；本機閘道認證
Codex 設定套⽤／還原 必備 路徑偵測＋備份＋原⼦寫入＋外部修改偵
測
條款／公告提⽰ 必備 登入前可⾒；認證警告則停
⽤量⼤盤／多⼯具 MCP 不做 超出取代雙開的最⼩範圍
⾃動 failover 不做（預設） 避免靜默換模掩蓋錯誤
暴露 Antigravity 的 Claude 不做 範圍僅 ChatGPT＋Gemini
Credential Manager／
Keychain 包裝
條件 僅上游整合 PoC 通過後；否則誠實標⽰上
游檔案型儲存

## Page 9

2026/11/7 上午10:30APP-AGG-001 v1 聚合閘道架構與功能規劃
第9⾴（共11⾴）https://app-agg-001-v1-e35b.pages.grok.bot/
MVP 資訊架構（中性⽰意）
以下為產品資訊架構，不是任何廠商 UI 仿製。
1.⾸⾴／狀態
a.閘道：停⽌／啟動中／健康／降級
b.⽬前 OS／架構標籤與 core checksum 短碼
c.監聽：127.0.0.1 與埠號策略
d.⽬前套⽤到 Codex 的 provider 摘要（無密⽂）
2.帳號
a.新增 ChatGPT／Codex 登入
b.新增 Antigravity 登入
c.登出／撤銷、上次成功時間、警告狀態
3.模型
a.聚合清單（來源標籤、狀態、別名、實際 id）
b.⼿動選為⽬前模型
c.對選定模型執⾏驗證回合
4.Codex 連結
a.偵測並顯⽰設定路徑（各平台待確認）
b.預覽將寫入的安全 diff
c.套⽤／還原／外部修改衝突處理
5.設定與關於
a.埠策略、單實例、退出⾏為
b.core 版本／checksum／架構
c.授權與來源連結、條款提⽰確認時間
OAuth ⽣命週期（概念）
1 提⽰條款
顯⽰服務商／上游限制提醒；使⽤者確認後才繼續
2 啟動上游 OAuth
開啟系統瀏覽器；state／PKCE／localhost callback 或 device flow 依上游實作
3 回呼完成
僅顯⽰帳號別名與狀態；token 不進 UI／⽇誌

## Page 10

2026/11/7 上午10:30APP-AGG-001 v1 聚合閘道架構與功能規劃
第10⾴（共11⾴）https://app-agg-001-v1-e35b.pages.grok.bot/
僅顯⽰帳號別名與狀態；token 不進 UI／⽇誌
4 失敗即停
認證失敗或帳號／登入警告 → 中⽌並提⽰，不重試規避
5 撤銷
本機登出＋依上游能⼒撤銷；清掉本⼯具側引⽤
階段與驗證閘⾨
PoC（進⾏中） MVP（未開） 強化
PoC 閘⾨（進⾏中 · macOS／Windows 必過）
規劃核准＋Phase 1 開⼯（私⼈ Origin 草稿雲端開發，無公開發
⾏）
2026-11-06 12:13
程式開發已啟動（含 Linux 評估導向的共⽤邏輯） 進⾏中·無測試結果
macOS＋Windows：ChatGPT／Codex 與 Antigravity OAuth 皆完成 未回報
macOS＋Windows：兩帳號各完成真實串流 coding／tool turn 未回報
macOS＋Windows：桌⾯ Codex 設定路徑與重啟條件確認 未回報
必驗架構 sidecar 產物／checksum 盤點；缺者 unavailable 未回報
Linux 評估：基線 distro／版本／架構、WebKitGTK、tray、Secret
Service／無 keyring ⾏為
評估中·非正式⽀援
未過 macOS＋Windows PoC 閘⾨不得核准正式／MVP 發⾏。單⼀必驗 OS 通過不算完
成。Linux 評估結果不替代必驗平台 native 證明。
驗收分層（禁⽌混報）
層級 含義
build-pass 該 OS／架構編譯／包裝成功
unit-pass 模組單元測試通過
protocol-pass Responses／OAuth 回呼等協議層測試

## Page 11

2026/11/7 上午10:30APP-AGG-001 v1 聚合閘道架構與功能規劃
第11⾴（共11⾴）https://app-agg-001-v1-e35b.pages.grok.bot/
integration-pass 與釘選 core 的整合測試（標明架構）
ui-verified-native-desktop 在該 OS 原⽣桌⾯實機操作驗證（禁⽌⽤ Wine 代
替 Windows；禁⽌⽤未跑測試充通過）
輕量指標（記憶體、啟動時間、安裝體積）須先在兩 OS 量測基準，再由使⽤者核准⽬標
數字；本⾴不發明數字。
埠與閘道策略
1.優先 loopback；預設拒絕非本機
2.穩定埠（操作簡單）與隨機埠（減少衝突）兩選項並存：MVP 採可設定穩定埠＋佔⽤
偵測；PoC 可加測隨機埠
3.本機閘道需本地 auth（僅本機可知的 bearer）；不上游 token 充當閘道密碼寫入可讀
⽇誌
安全與合規
1.永不收集或要求密碼；OAuth 只走系統瀏覽器與上游⽀援回呼
2.⽇誌與 UI 禁⽌輸出 access／refresh token、cookie、授權碼
3.認證失敗或帳號／登入警告：停⽌該流程並提⽰，不繞過
4.憑證儲存：預設沿⽤上游機制；Windows Credential Manager／DPAPI、macOS
Keychain、Linux Secret Service 僅為擬議適配器，須 PoC 證明可接上游後才可承諾
5.Linux 無 Secret Service／headless：明⽰失敗或安全降級，禁⽌靜默明⽂寫入
6.若上游必須寫入磁碟檔：⽂件與 UI 明確標⽰，不承諾「密鑰永不落地」
7.Codex 設定：不覆寫無關鍵與他⼈憑證欄；衝突時安全 diff＋使⽤者確認
8.與 CC Switch 並存：明確擁有權；偵測外部編輯；提供還原
風險與 Go／No-Go
風險 影響 緩解
某 OS 桌⾯ Codex 設定路徑
／重啟不明
該平台套⽤無效 分平台 PoC；未確認平台不標 MVP 通過