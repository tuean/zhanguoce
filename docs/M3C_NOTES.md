# M3c-1 · 通鑑戰國段事件表與年表頁

狀態：已完成。源 ＝ 資治通鑑 周紀一～五 + 秦紀一～二（前403–前209）。

## 1 · 資料從哪來

| 檔案 | 卷 | 覆蓋 | div.yt | 事件 |
| --- | --- | --- | --- | --- |
| 5282.html | 周紀一 | 前403–前369 | 30 | 103 |
| 5381.html | 周紀二 | 前368–前321 | 40 | 107 |
| 5382.html | 周紀三 | 前320–前298 | 23 | 75 |
| 5383.html | 周紀四 | 前297–前273 | 25 | 73 |
| 5384.html | 周紀五 | 前272–前256 | 16 | 49 |
| 5385.html | 秦紀一 | 前255–前228 | 28 | 94 |
| 5386.html | 秦紀二 | 前227–前209 | 18 | 82 |
| 合計 | | 195 年 | **180** | **583** |

- 抓取用 **GitHub contents API + base64**（tools/fetch_tongjian.py）：raw.githubusercontent.com 會成批超時（curl (28) after 20005ms），API 每檔一次、串行、未認證 60 次/小時，只花 7 次。
- 緩存落在 tools/_cache/tongjian/；已存在即跳過，所以重跑不再聯網，離線也能重建。

## 2 · 源結構與解析規則（tools/build_events.py）

實測結構（與 EVENT_SOURCES.md 一致，另補三條實作細節）：

1. div class=yt id=y0403 裡**只有年頭那一個** p class=strong：span.section（data-head-description＝王、data-head-title＝年）＋ span.biaozhusup（干支、前X年）＋ 年注 span.jiazhu。
2. **事件正文不在 div.yt 裡**，而是它之後的兄弟 p class=em2 strong id=pN，一直排到下一個 div.yt。這與「div 內含全部段落」的直覺不同，是第一處坑。
3. p class=margin2 是「臣光曰」議論，**不入事件表**（不是紀事）。空 class 的 p 是卷首／王名注，也排除。
4. 部分事件段沒有 span.yinwen 序號（26/24/14/16/16/13/44），它們仍是事件 —— 不能用「有無序號」判斷。
5. 卷名只有 5282 的 h1 帶 data-head-description，其餘為空，故一律取 h1 的 data-head-title 去非漢字（5383 原文是 周紀四&lt;），再與檔案→卷名映射斷言相等。
6. 文字抽取用 span 棧：jiazhu→注文槽、jiaozhusup→校記槽、yinwen→丟棄、其餘 span（shu-ming／yinhao／section／biaozhusup／pass）透明。**第二處坑**：透明的 span 若直接落回正文槽，注文裡的 span.shu-ming（例如「通鑑」）會漏進正文；修法是從棧頂向下找最近的注文／校記／丟棄槽再落正文。

## 3 · 檔案結構（data/zhanguoce/events/tongjian.json）

~~~
{ schemaVersion: 1, bookId: "zhanguoce", source: "資治通鑑·周紀/秦紀（戰國段）",
  generatedBy: "tools/build_events.py", years: [...], events: [...] }
~~~

- years[]：一年一條 div.yt 記錄 —— { yearBc, div, label, ganzhi, ruler, rulerYear, juan, sourceFile, anchor, note }。label 保留原樣（（戊寅、前四○三年））。
- events[]：一段一條 —— { id, yearBc, ruler, rulerYear, ganzhi, juan, sourceFile, anchor, text, note, collation }。
- **同一年可有多條事件**（一個 div.yt 下多個 p），共享年資訊；事件不是年份的子物件，而是平行陣列，靠 yearBc 連接，這樣兩個方向都能逐條交叉驗證。
- 事件冗餘存 ruler/rulerYear/ganzhi/juan/sourceFile：內核才能斷言「每一條事件的年份欄位與它所屬年份記錄一致」，否則只剩 builder 端的自我保證。
- id ＝ tj-<div>-<anchor>（例如 tj-y0403-p4），穩定且可由年份＋錨點重算 —— 內核會驗證這個等式，id 一改就報錯。
- text 是正文（去 a 錨點、span.yinwen 序號與全部標籤），note 是 span.jiazhu 注文（無則 null），collation 是 span.jiaozhusup 校記（無則 null，全表 93 條）。
- **繁簡不轉**：正文與注文一律保留底本繁體。全表 0 空正文、0 標籤殘留、最長 1005 字、平均 101 字、56 條無注。

## 4 · 自校驗（構建期硬失敗）

tools/build_events.py 每條斷言不成立即非零退出、不寫檔案；crates/zgc-core/src/events.rs 的 Events::build 對前五條再做一次同樣的拒絕（同一份資料，兩個獨立實作互相監督）。

| # | 斷言 | builder | 內核 |
| --- | --- | --- | --- |
| 1 | yearBc ＝ div id 的 yNNNN | PASS 180/180 | DivYearMismatch |
| 2 | yearBc ＝ 標籤「前X年」（三位中文數字，○／〇／零 歸一） | PASS 180/180 | LabelMismatch |
| 3 | ganzhi ＝ 標籤干支 | PASS 180/180 | LabelMismatch |
| 4 | ruler／rulerYear 來自 data-head-* | PASS 180 年頭 · 583 事件 | 非空 ＋ 事件與年份記錄逐欄一致（EmptyField／YearFieldMismatch） |
| 5 | id 唯一、anchor 在源 div 內真實存在 | PASS 583 個唯一 id · 583 anchor | DuplicateEventId ＋ id ＝ tj-<div>-<anchor>（IdShapeMismatch）＋ anchor 非空 |
| 6 | 匯總：div.yt 180 · 事件 583 · 前403–前209 · 無事件年份 15 | PASS | 頁面與啟動日誌複核 |

內核看不到 HTML，所以「anchor 在源 div 內真實存在」由 builder 負責；內核改用 id 形狀等式把 id、div、anchor 三者綁死，任何一處漂移都會被拒絕。

無事件年份（15）：217, 261, 331, 339, 342, 345, 349, 360, 363, 367, 374, 384, 388, 392, 395。

## 5 · 內核（crates/zgc-core/src/events.rs）

- Events::load(book_dir) 讀 events/tongjian.json；**檔案不存在 → Ok(Events::empty())**，不報錯也不 panic（將來的古籍沒有年表是常態），讀檔或 JSON 壞掉才 Err。
- Events::build(EventFile) 是唯一驗證器，load 與程式化構造共用。
- 降序持有：years() 與 events() 都是前403 → 前209，且**同一年的事件在 events() 裡連續**，所以 events_of_year(y) 能直接切 slice，不必每次重分組。
- 其他：event(id)、year(y)、year_bounds() 回 (403, 209)（larger BCE ＝ 更早，故 earliest 取 max）、years_without_events()、find(needle)（正文／注／校記子串）、year_count()、event_count()、is_empty()。
- App::with_events(data, entities, events) 掛上時間軸；App::with_entities 保留給測試（#[cfg(test)]），伺服器一律走 with_events。啟動日誌多一行：事件：180 年 · 583 條（前403–前209）· 15 年無條目 · 見 /timeline。

## 6 · 頁面（/timeline）

- ?y=403（也接受 前403）：只渲染該年。?e=tj-y0403-p4：渲染該事件所屬年份並標紅該條，因此 ?e= 與 /timeline#tj-y0403-p4 都能直達。?q=：正文／注／校記子串過濾，命中處用 mark，可與 ?y= 疊加。
- **年份降序**（前403 在最上、前209 在最下）：戰國段讀者多是從「三家分晉」往後看，且新資料更常被查；載入時即排好，頁面只切片。
- 每條事件是 article class=tl-ev id=tj-y0403-p4，注與校記放原生 details（零 JavaScript）；年份塊 section class=tl-year id=y0403 也可被錨定。
- 參數壞掉不 404：年份解析失敗或查無事件，都在頁內 p.notice 說明後列出全部 —— 年表是「查的地方」，不是「可能缺失的文件」。
- 樣式只往 crates/zgc-app/src/style.css 追加 .tl-* 段，沿用既有 --paper／--rule／--accent／--card 變數。

## 7 · 已知取捨

- 「臣光曰」議論（p.margin2）不入事件表：它是評論而非紀事，硬塞進去會讓「事件數」變成兩種東西的混合。
- 校記（span.jiaozhusup）單列 collation 而不併進注文：來源不同、讀者用途不同。
- ?q= 用簡單子串而非 bigram：事件表 583 條（35 萬字符），線性掃描不到 1 ms；bigram 索引是段落級的，對事件沒有現成映射。等事件表長到幾萬條再換。
- 事件與本專案的段 id（dongzhou-01-p0001 等）**沒有**對應關係 —— 通鑑不在 11 國策語料裡，sourceFile／anchor 指向的是上游通鑑 HTML，不是本倉庫的段落。
- 上游 repo 無 LICENSE，古籍原文屬公有領域，整理本僅作內部研究用；本表只存原文與注文，不再分發上游檔案（tools/_cache/ 為抓取緩存，可刪）。

## 8 · 重現

~~~bash
python3 tools/fetch_tongjian.py     # 首次或清緩存後；7 次 API 請求
python3 tools/build_events.py       # 解析 + 6 條自校驗 + 寫 JSON（冪等）
cargo test --workspace
target/release/zgc-app --port 8099  # /timeline
~~~
