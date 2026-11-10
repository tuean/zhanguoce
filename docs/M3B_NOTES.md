# M3B · 史事層接進閱讀器

本階段把 `data/zhanguoce/entities/*.json` 的 274 個實體（人物 176、地名 98）與
10,685 條出現記錄接進 SSR 閱讀器：原文列可點、實體頁可回溯、索引頁可瀏覽。
數據文件一字未改，所有校驗在載入期完成。

## 分層

- **內核** `crates/zgc-core/src/entities.rs`（新增）：類型、載入、校驗、查詢。
  公開 `EntityKind / Entity / Occurrence / SpanView / Entities / EntityError /
  EntityFile / EntityRecord / OccurrenceRecord`，由 `lib.rs` 轉出。
- **渲染** `crates/zgc-app/src/{app,pages}.rs`：只消費內核 API，不碰 JSON。
- 啟動時一次建索引：`Entities::load(&data_dir, &data)`，之後每次請求只查
  `spans_in(pid)` / `entity(id)`；掃 10,685 條只發生一次。

## 載入期 fail-fast

複刻 `docs/ENTITIES.md` 裡可機械檢查的部分，任一不滿足即整體失敗（不靜默丟棄）：

- id 非空且唯一；kind 與文件名一致；name 非空；每個實體至少一條出現記錄；
- 段 id 必須存在於正史數據；offset/len 必須落在該段字符長度之內；
- `form` 必須是該實體的 name 或 aliases 之一；
- **字符級**比對 `text[o..o+len] == form`（按 `chars()` 切片，絕不用字節）。

## 三條硬邊界

1. **重疊 span**：`resolve()` 先按 `len` 降序、`offset` 升序、實體序號升序排序，
   再貪心接受互不衝突者、丟棄所有衝突項，最後按 `(offset, len)` 重排。
   因此同一偏移最長匹配勝出、短者整條丟棄，輸出永不嵌套且確定。
   真實語料 4,195 個含實體段裡有 38 段發生重疊（例如 `東周君` 與 `東周` 同起點），
   全部由上述規則取捨。
2. **轉義不破**：文本一律先 `html::escape`，再拼 `<a>`/`<mark>`。
   `/read?p=<script>alert(1)</script>` 仍返回 404 且正文只有 `&lt;script&gt;`。
3. **偏移是字符**：`char_slice()` 用 `chars()` 索引；單測用「秦」😀衞鞅 驗證
   `char_indices().nth(4)` 得到字節 13（即字節偏移會錯位）。

## 路由與頁面

| 路由 | 行為 |
| --- | --- |
| `/read?p=<pid>&hl=<o>,<len>` | 原文列把命中實體渲染成 `<a class="ent ent-<kind>">`；`hl` 合法且與 span 相交時加 `ent-hl` 並在鏈接內套 `<mark>`。`hl` 缺省/非數字/長度為 0/越界一律當作沒有，不報錯、不 panic。 |
| `/entity?id=<entity-id>` | name / nameSimp / aliases / tags / summary（null 或空白顯示「暫無」），出現處按 卷→章 分組，每條 `/read?p=<pid>&hl=<o>,<len>`；未知 id 404，缺 id 400。 |
| `/entities[?kind=person\|place][&sort=count\|name]` | 274 個實體的出現次數列表；kind 過濾、count/name 排序，未知 kind 給提示並列全部。 |
| `/search` | 每條命中附該段包含的實體鏈接（可選項，已做）。 |

頁面零 JavaScript，全部服務端渲染；導航欄統一加入「實體索引」入口。

## 性能（release，本機，300 次請求）

| 頁面 | body | median | p95 |
| --- | --- | --- | --- |
| `/read?p=qin-1-01-p0001`（改前） | 13,868 B | 0.147 ms | 0.215 ms |
| `/read?p=qin-1-01-p0001`（改後） | 16,234 B | 0.144 ms | 0.226 ms |
| `/entity?id=person-wei-yang` | 18,058 B | 0.124 ms | 0.174 ms |
| `/entities` | 58,892 B | 0.179 ms | 0.254 ms |

體積增大來自 6 個新鏈接，耗時在噪聲範圍內（索引只建一次，請求期只做切片+拼接）。

## 測試

- `crates/zgc-core/src/entities.rs` 12 個單測：字符偏移（含 emoji、全角標點）、
  重疊取最長且 8 次重跑一致、相鄰不誤傷、`text[o..o+len] != form` 被拒、
  空/重複/未聲明/類型不符記錄被拒、null 與空白 summary 視同缺失、
  出現記錄按閱讀順序、kind 過濾不亂序、空索引不 panic。
- `crates/zgc-core/tests/entities_data.rs` 5 個真實數據測試：
  274/10,685/4,195/38、`person-wei-yang` 23 次、`place-qin` 1,910 次、
  每條出現與每個渲染 span 都能切回 name/alias、span 互不重疊且在界內、
  東周同點位由東周君勝出、兩次載入逐字節一致。
- `crates/zgc-app` 新增 15 個渲染測試（含 5 個真實語料）：鏈接位置、嵌套為零、
  詳情頁字段與「暫無」、轉義、404/400、索引排序與過濾、非法 `hl` 清單、
  XSS 反證、真實重疊段每 span 一個錨點。

`cargo test --workspace` 共 136 個測試全綠（既有 104 + 新增 32）；
`cargo build --release` 0 warning。