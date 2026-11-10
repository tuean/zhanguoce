# M2b · 研读阅读器（服务端渲染，零前端框架）实现说明与实测

> 目标：只用 Rust std + `zgc-core` 起一个只监听 127.0.0.1 的极简 HTTP 服务，服务端渲染 HTML，
> 实现《戰國策》的目录 / 阅读 / 检索 / 进度记忆。**不引前端框架、不引 Web 框架、页面零 JavaScript。**

## 1. 交付物与命令

    Cargo.toml                     workspace members 增加 crates/zgc-app（唯一改动的既有文件）
    crates/zgc-app/Cargo.toml      仅依赖 zgc-core（path）
    crates/zgc-app/src/main.rs     启动：参数/环境变量、加载、建索引、绑定、accept 循环、请求日志
    crates/zgc-app/src/http.rs     手写 HTTP/1.1：读请求头、解析请求行与头部、写响应
    crates/zgc-app/src/app.rs      App 状态（书 + 索引 + id→位置表）与路由
    crates/zgc-app/src/pages.rs    目录页 / 阅读页 / 检索页 / 404 / 400 的 HTML 拼装
    crates/zgc-app/src/prefs.rs    版面、原文/白话字号、行距的解析与持久化
    crates/zgc-app/src/urlcodec.rs 百分号编解码、query / cookie 解析
    crates/zgc-app/src/html.rs     HTML 转义 + 命中高亮
    crates/zgc-app/src/style.css   手写样式（include_str! 内联进页面）

运行与测试：

    cargo run --release -p zgc-app                       # 默认 127.0.0.1:8099，数据 data/zhanguoce
    cargo run --release -p zgc-app -- --port 8137 --data-dir data/zhanguoce
    ZGC_DATA_DIR=/path/to/book ZGC_PORT=8200 cargo run --release -p zgc-app
    cargo test --workspace

## 2. 技术路线落地（逐条对照硬性要求）

| 要求 | 实现 |
|---|---|
| 不引前端框架 / wasm / npm | 页面是纯 HTML+CSS，**零 JavaScript**；workspace 里没有任何前端相关 crate |
| 不引 Web 框架 | `std::net::TcpListener` + `std::io::{Read,Write}`，只支持 GET，读到头部空行即返回 |
| CSS 内联、不引模板引擎 | `include_str!("style.css")` 直接写进 `<style>`，HTML 用 `format!` 拼装，全部经过转义函数 |
| 依赖只新增 zgc-core | `cargo tree -p zgc-app` 只有 zgc-core（serde / serde_json 是内核既有传递依赖） |
| 自己写百分号编解码 | `urlcodec.rs`：解码永不失败（坏转义原样保留，非法 UTF-8 降级 U+FFFD），按 RFC 3986 unreserved 编码 |
| 只监听 127.0.0.1、端口可配置 | `TcpListener::bind(("127.0.0.1", port))`，默认 8099，`--port` / `ZGC_PORT` 覆盖 |
| 端口占用给清晰错误、不 panic | 见 §6.5：AddrInUse → 说明端口 + 给替代端口，退出码 1 |

连接模型：accept → 每连接一个线程（`Arc<App>` 只共享只读状态）→ 读一个请求头 → 写一个响应 →
`Connection: close` 关闭。读超时 5 s、写超时 10 s，请求头上限 16 KiB（超限 431）。

## 3. 路由

| 路由 | 行为 |
|---|---|
| `GET /` | 目录页：国(group) → 卷 → 章 有序树，卷序严格按 volumeOrder（经 group_volume_indices），并含「繼續閱讀」 |
| `GET /read?p=<段id>` | 阅读页：三栏原文/白话/注疏，版面 + 字号 + 行距切换，段/章/卷 pager，面包屑定位 |
| `GET /search?q=…&group=<group-id>` | 检索页：命中条数 + 卷/章/第几段 + 高亮片段 + 进阅读页；group 走 `Scope::Group` |
| `GET /favicon.ico` | 204（避免日志噪音） |
| 其他路径 | 404 页面 |
| 非 GET | 405 + `Allow: GET` |

阅读页 query 参数（都可选；缺省取 cookie，再取默认）：

| 参数 | 取值 | 说明 |
|---|---|---|
| p | 段 id | 必填；查不到 → 404 页面；缺失 → 400 页面 |
| layout | tri / dual / single | 三栏 / 双栏（原文+白话） / 单栏（仅原文） |
| orig | s / m / l / xl | 原文字号 1.10 / 1.28 / 1.46 / 1.66 rem |
| tran | s / m / l / xl | 白话字号 0.92 / 1.00 / 1.10 / 1.22 rem（与原文独立） |
| lh | tight / normal / loose | 行距 1.9 / 2.08 / 2.3（**全部 ≥ 1.9**） |

Cookie：`zgc_last`（进度，HttpOnly）、`zgc_layout`、`zgc_orig`、`zgc_tran`、`zgc_leading`（一年期）。
query 参数优先，只要出现就回写 cookie；pager 与章链接只带 p，设置靠 cookie 延续。
窄屏（≤880px）一律 `grid-template-columns: 1fr`：三栏/双栏降为单栏堆叠，单栏仍只显示原文。

## 4. 与 zgc-core 的接口使用（内核未改动一行）

- 加载：`Book::load(&Path)` → `BookData`
- 结构：iter_positions / paragraph / chapter / volume / volume_by_id / volume_start / group / groups /
  group_volume_indices / group_count / volume_count / chapter_count / paragraph_count
- 导航：全部用 next_paragraph / prev_paragraph / next_chapter / prev_chapter / next_volume / prev_volume，
  UI 侧不自己算任何边界
- 检索：`SearchIndex::build` 一次 + `search(&data, q, &Scope::{All, Group})`

内核之外补的一层（在 `App::new` 里，**启动时建一次**，不在渲染路径上）：
段 id → Position、章 id → 首段 Position 两个 HashMap（6663 段，约 1 ms）。
渲染时只查表 + 拼字符串：**每次请求都不重建索引**。

## 5. 安全与健壮性

- 转义：所有来自数据与请求的文本都过 `html::escape`（& < > " ' 全转义）；检索片段用
  `html::escape_highlight`（先转义再插 `<mark>`，按 char 走，不会切碎多字节字符）。
- 用户输入校验：段 id 只用于查表（查不到 404）；layout/orig/tran/lh 只接受白名单键值，非法值忽略；
  group 只有与 book.groups 匹配时才进 `Scope::Group`，否则退回全書并在页面提示。
- 长度上限：请求头 16 KiB、检索词 64 字、404 回显 160 字、日志 target 120 字。
- 不 panic：未知段 id / 未知路径 / 非 GET / 空 p 都返回状态页；`--port abc` 给退出码 2。

## 6. 实测证据

### 6.1 构建与测试

    $ cargo build --release
       Compiling zgc-app v0.1.0 (/Users/zhongxiaotian/RustroverProjects/this/crates/zgc-app)
        Finished release profile [optimized] target(s) in 0.64s
    $ cargo build --release 2>&1 | grep -ci warning
    0

    $ cargo test --workspace
    running 52 tests   (zgc-app：路由/渲染/转义/HTTP/参数/编解码 + 真实语料渲染)
    test result: ok. 52 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
    running 32 tests   (zgc-core src 单元)
    test result: ok. 32 passed; 0 failed
    running 7 tests    (zgc-core 真实数据集成)
    test result: ok. 7 passed; 0 failed
    running 12 tests   (zgc-core 加载器夹具)
    test result: ok. 12 passed; 0 failed
    Doc-tests zgc_core
    running 1 test
    test result: ok. 1 passed; 0 failed
    （zgc-app 52 + zgc-core 52 = 104 通过 / 0 失败。内核的测试数由内核任务独立维护，
      会随其改动变动；本次 zgc-app 侧为 52 个，见 §6.1 首段。）

启动日志（release，本机）：

    書目：戰國策（zhanguoce）
    結構：11 國 · 33 卷 · 497 章 · 6663 段
    索引：129479 個 bigram / 471435 次出現（啟動時建一次，之後每次請求只讀）
    耗時：load 16.08 ms · index 38.41 ms
    服務：http://127.0.0.1:8137/

### 6.2 四个必测场景（curl 实测）

① 目录页

    $ curl -s -o toc.html -w 'HTTP %{http_code} · %{size_download} B · %{time_total}s\n' http://127.0.0.1:8137/
    HTTP 200 · 50512 B · 0.001137s

    $ grep -o '<p class="stats">[^<]*</p>' toc.html
    <p class="stats">11 國 · 33 卷 · 497 章 · 6663 段</p>
    $ grep -o '<span class="g-title">[^<]*…' toc.html | head -3
    <span class="g-title">東周<span class="simp">（东周）</span></span>
    <span class="g-title">西周</span>
    <span class="g-title">秦策</span>
    $ grep -o 'href="/read?p=[a-z0-9-]*"' toc.html | head -3
    href="/read?p=dongzhou-01-p0001"      ← 卷标题（卷首段）
    href="/read?p=dongzhou-01-p0001"      ← 该卷首章（同一段）
    href="/read?p=dongzhou-02-p0001"

首卷是 dongzhou（volumeOrder[0]），不是文件名字典序。全书 530 个阅读链接 = 33 个卷标题 + 497 个章节，
无重复（宋衛并卷已修，见 §8.1；测试同时断言卷/章链接数 == volume_count/chapter_count）。

② 阅读页

    $ curl -s -o read.html -w 'HTTP %{http_code} · %{size_download} B · %{time_total}s\n' \
        'http://127.0.0.1:8137/read?p=qin-1-01-p0001'
    HTTP 200 · 13868 B · 0.000671s

三部分都在同一份 HTML 里（grep 原文 / 白话 / 注疏）：

    $ grep -o '<h2>原文（繁體）</h2>' read.html
    <h2>原文（繁體）</h2>
    $ grep -o '<p class="orig-line">[^<]*</p>' read.html
    <p class="orig-line">衞鞅亡魏入秦，孝公以爲相，封之于商，號曰商君。</p>
    $ grep -o '<h2>白話（簡體）</h2>' read.html
    <h2>白話（簡體）</h2>
    $ grep -o '<p class="tran">[^<]*</p>' read.html
    <p class="tran">卫鞅从魏国逃亡到秦国，秦孝公任命他为丞相，把商地分封给他，号称 商君 。</p>
    $ grep -o '<h2>注疏<span class="count">[0-9]*</span></h2>' read.html
    <h2>注疏<span class="count">5</span></h2>
    $ grep -o '<span class="src">[^<]*</span><p class="note-text">[^<]*</p>' read.html | head -2
    <span class="src">鲍彪</span><p class="note-text">衛之庶孳公子姓公孫氏</p>
    <span class="src">鲍彪</span><p class="note-text">鞅事衛相公叔痤為庶子見魏䇿</p>

定位与导航（同一页）：

    $ grep -o '<span class="pos">[^<]*</span>' read.html
    <span class="pos">第 1 段 · 共 11 段</span>
    面包屑：目錄 / 秦策（国策名） / 秦策一（卷名） / 衞鞅亡魏入秦（章名） / 第 1 段 · 共 11 段
    pager：← 上一段（首段 → disabled「已是首段」）、下一段 →、⇤ 上一章（首章 → disabled）、
           下一章 ⇥、⌃ 上一卷（首卷 → disabled）、下一卷 ⌄ —— 目标全部来自 zgc-core 的 nav API

空注疏 / 空白话也正确（不是空白）：

    $ grep -o '暫無注疏\|暫無白話' 该页
    暫無注疏
    暫無白話

③ 检索页

    $ curl -s -o search.html -w 'HTTP %{http_code} · %{size_download} B\n' \
        'http://127.0.0.1:8137/search?q=%E8%A1%9E%E9%9E%85'      # q = 衞鞅
    HTTP 200 · 12430 B

    $ grep -o '「<span class="q">[^<]*</span>」命中 [0-9]* 條[^·]*· 範圍：[^<]*' search.html
    「<span class="q">衞鞅</span>」命中 5 條 · 範圍：全書
    $ grep -o '<mark>[^<]*</mark>' search.html | head -3
    <mark>衞鞅</mark>  ×3
    $ grep -o 'href="/read?p=[^"]*"' search.html | head -3
    href="/read?p=qin-1-01-p0001"
    href="/read?p=qi-5-01-p0083"
    href="/read?p=qi-5-01-p0085"

限定国策（Scope::Group）：同一个词换范围，命中数不同，且结果全部落在该组卷内：

    $ curl -s 'http://127.0.0.1:8137/search?q=%E7%A7%A6%E7%8E%8B&group=qi'    # q=秦王, group=qi
    「<span class="q">秦王</span>」命中 34 條 · 範圍：齊策
    $ grep -o 'href="/read?p=[a-z]*' 该页 | sort -u
    href="/read?p=qi                                                          （只有齊策各卷）
    $ curl -s 'http://127.0.0.1:8137/search?q=%E7%A7%A6%E7%8E%8B'
    命中 607 條（僅顯示前 200 條） · 範圍：全書

④ 不存在的段 id → 404

    $ curl -s -o nf.html -w 'HTTP %{http_code} · %{size_download} B\n' \
        'http://127.0.0.1:8137/read?p=qin-9-99-p9999'
    HTTP 404 · 10611 B
    $ grep -o '找不到：<code>[^<]*</code>' nf.html
    找不到：<code>qin-9-99-p9999</code>
    $ grep -o '<h1 class="page-title">[^<]*</h1>' nf.html
    <h1 class="page-title">找不到這一段</h1>

### 6.3 版面 / 字号 / 行距 / 进度（实测）

    $ for L in tri dual single; do curl -s "/read?p=qin-1-01-p0001&layout=$L" | grep -o 'class="cols layout-[a-z]*"'; done
    class="cols layout-tri"
    class="cols layout-dual"
    class="cols layout-single"

    $ curl -s -H 'Cookie: zgc_layout=dual; zgc_orig=xl; zgc_leading=loose' '/read?p=qin-1-01-p0001' | grep -o …
    class="cols layout-dual"
    <html lang="zh-Hant" style="--orig-size:1.66rem;--tran-size:1.00rem;--leading:2.3"

    $ curl -s -D - -o /dev/null '/read?p=qin-1-01-p0001&orig=l&tran=s&layout=dual&lh=tight' | grep -i '^set-cookie'
    Set-Cookie: zgc_last=qin-1-01-p0001; Path=/; Max-Age=31536000; SameSite=Lax; HttpOnly
    Set-Cookie: zgc_layout=dual; Path=/; Max-Age=31536000; SameSite=Lax
    Set-Cookie: zgc_orig=l; Path=/; Max-Age=31536000; SameSite=Lax
    Set-Cookie: zgc_tran=s; Path=/; Max-Age=31536000; SameSite=Lax
    Set-Cookie: zgc_leading=tight; Path=/; Max-Age=31536000; SameSite=Lax

    $ curl -s -H 'Cookie: zgc_last=qin-1-01-p0001' http://127.0.0.1:8137/ | grep -o '<section class="resume">…'
    <section class="resume"><h2>繼續閱讀</h2><p><a href="/read?p=qin-1-01-p0001">秦策一<span class="sep">·</span>衞鞅亡魏入秦<span class="sep">·</span>第 1 段</a></p></section>

（cookie 里的段 id 若已失效，首页不显示「繼續閱讀」，也不报错。）

### 6.4 渲染耗时

每次响应都打印 render（handler 内耗时）与 total（含 socket）：

    GET /read?p=qin-1-01-p0001           render 0.02–0.07 ms · total 0.04–0.09 ms · body 13868 B
    GET /search?q=秦王&group=qi          render 0.37 ms      · total 0.81 ms · body 23207 B
    GET /search?q=秦王（200 条，83 KB）   render 0.82 ms      · total 1.70 ms · body 84771 B

（时间为服务端自记的 render / total；字节来自 curl 的 size_download，只算 body。）

curl 端到端（含 TCP 连接）连打 30 次阅读页：

    n=30  min=0.320 ms  avg=0.355 ms  max=0.437 ms

对照启动日志：建索引 38.4 ms 只发生一次；单页 0.02–0.07 ms 量级说明渲染路径上没有索引工作。

### 6.5 错误路径（不 panic）

    $ ./target/release/zgc-app --port 8137 --data-dir data/zhanguoce       # 端口已被占用
    錯誤：端口 8137 已被佔用（可能已有一個 zgc-app 在跑，或系統佔用了該端口）。
    換一個端口：zgc-app --port 8138 或 ZGC_PORT=8138 zgc-app
    exit=1

    $ ./target/release/zgc-app --port 8138 --data-dir data/does-not-exist
    錯誤：讀取書目失敗（data/does-not-exist）：读取 …/book.json 失败：No such file or directory
    提示：用 --data-dir 或 ZGC_DATA_DIR 指向含 book.json 與 volumes/ 的目錄。
    exit=1

    $ ./target/release/zgc-app --port abc
    參數錯誤：端口必須是 1–65535 的整數，收到「abc」     exit=2

    $ curl -s -o /dev/null -w '%{http_code}' -X POST '/read?p=qin-1-01-p0001'   → 405 (+ Allow: GET)
    $ curl -s '/nope'                                                          → 404

XSS 实测（两条都只剩转义后的文本）：

    $ curl -s '/read?p=%3Cscript%3Ealert(1)%3C/script%3E' | grep -o '找不到：<code>[^<]*</code>'
    找不到：<code>&lt;script&gt;alert(1)&lt;/script&gt;</code>
    $ curl -s '/read?p=%3Cscript%3Ealert(1)%3C/script%3E' | grep -c '<script>alert'
    0
    $ curl -s '/search?q=%3Cscript%3Ealert(1)%3C/script%3E' | grep -o '<span class="q">[^<]*</span>'
    <span class="q">&lt;script&gt;alert(1)&lt;/script&gt;</span>        （grep -c '<script>alert' → 0）

## 7. 依赖清单与新增文件

`cargo tree -p zgc-app`：

    zgc-app v0.1.0 (crates/zgc-app)
    └── zgc-core v0.1.0 (crates/zgc-core)
        ├── serde v1.0.229
        │   └── serde_core / serde_derive → proc-macro2 / quote / syn / unicode-ident
        └── serde_json v1.0.154
            └── itoa / memchr / serde_core / zmij

**新增依赖：0**（除 workspace 内 zgc-core 路径依赖；serde 系是内核原有传递依赖）。
**没有** leptos / dioxus / yew / wasm / axum / actix / hyper / tiny_http / url / maud / askama。

新增 / 改动文件：

    M  Cargo.toml                            （members 增加 "crates/zgc-app"）
    A  crates/zgc-app/Cargo.toml             (15 行)
    A  crates/zgc-app/src/main.rs            (326 行)
    A  crates/zgc-app/src/app.rs             (472 行)
    A  crates/zgc-app/src/pages.rs           (521 行)
    A  crates/zgc-app/src/http.rs            (315 行)
    A  crates/zgc-app/src/prefs.rs           (318 行)
    A  crates/zgc-app/src/urlcodec.rs        (178 行)
    A  crates/zgc-app/src/html.rs            (88 行)
    A  crates/zgc-app/src/style.css          (335 行)
    A  crates/zgc-app/src/real_data_tests.rs (206 行)
    A  docs/M2B_NOTES.md                     （本文件）

data/**、tools/**、crates/zgc-core/**、docs/PLAN.md、docs/SCHEMA.md、docs/DATA_REPORT.md、
docs/ANNOTATION_SOURCES.md、docs/M2A_NOTES.md 均未改动。

## 8. 数据 / 内核观察（只报告；未改内核，未改数据）

1. **宋衛共卷（已修）**：M2b 首测时 book.json 里 song（宋策）与 weiwei（衛策）两个 group 都引用
   同一个卷 song-wei，目录树于是在两个组下各展开一次该卷（34 个卷位 / 33 个卷 / 497 章渲染出
   512 个章链接）。**数据侧随后已把 12 组合并为 11 组（宋衛策一组）**，复测：11 國 · 33 卷 ·
   497 章 · 6663 段，33 个卷标题链接 + 497 个章链接 = 530 个阅读链接，无重复。
   渲染仍按 group.volumes 忠实展开而不去重：若将来数据再出现「一卷归两组」，目录会如实展示两次，
   该不变量由 real_data_tests 的「卷/章链接数 == volume_count/chapter_count」守住。
2. **注疏用「衛」**：原文底本 5 处「衞鞅」，而鲍彪/吴师道注文写作「衛」。
   所以现在全库检索「衛鞅」有 **8 条命中（全部来自注疏字段，原文 0 条）**，而「衞鞅」5 条全在原文。
   M2a 记的「衛鞅 = 0」只对 text 字段成立。M3 若要做异体字 / 繁简归一，
   建议做在内核的 fold（索引与复核共用），UI 不用动。
3. **内核缺口（本次用启动期 map 绕开，未改内核）**：BookData 没有「段 id → Position」
   与「章 id → 首段 Position」的反查，而 SearchIndex 返回的正是段 id，UI 必须自己再定位。
   建议内核补 position_of(paragraph_id) / chapter_position_of(chapter_id)，语义与本次实现一致
   （本次实现：启动时遍历 iter_positions 建两个 HashMap，6663 段约 1 ms）。
   另外 BookData 有 volume_by_id，但没有对应的 chapter_by_id / paragraph_by_id。
4. **注疏 source 用字**：source 是简体（鲍彪 / 吴师道补 / 吴师道正），与繁体原文混排。
   页面按原样显示（转义后），不做繁简转换。属数据侧用字问题。
5. **检索计数是「出现次数」**：同段重复出现算多条（M2a 已说明）。阅读器按出现次数逐条渲染，
   单页最多显示 200 条并显示总数（「命中 607 條（僅顯示前 200 條）」）。

## 9. 假设（未逐条确认，按此推进；要改告诉我）

1. 默认端口 **8099**（非 80/443），可用 --port / ZGC_PORT 覆盖。
2. 默认数据目录 **data/zhanguoce**（相对启动目录），可用 --data-dir / ZGC_DATA_DIR 覆盖；
   一个进程一本书（多古籍切换后续再做）。
3. 偏好持久化用 cookie（zgc_layout / zgc_orig / zgc_tran / zgc_leading），query 参数优先并回写；
   pager 与章链接只带 p，设置靠 cookie 延续。
4. 进度 zgc_last（HttpOnly、一年）；首页「繼續閱讀」只在 cookie 里的段 id 仍可解析时出现。
5. 双栏 = 原文 + 白话（注疏列隐藏）；单栏 = 仅原文；≤880px 一律单列堆叠（版面选择仍决定哪些列存在）。
6. 行距三档 1.9 / 2.08 / 2.3（都 ≥ 1.9）；原文与白话各有 4 档字号且互相独立。
7. 一个 GET 一个连接（Connection: close），不实现 keep-alive / 分块传输 / 请求体；
   每连接一线程，无连接池（本机单用户场景足够）。

## 10. 本任务明确不做

- 章视图 / 卷视图独立页面（当前章、卷通过目录或 pager 跳到首段）
- 检索结果按段去重、真正的分页（现在是「前 200 条 + 总数」）
- 划词批注落盘、AI 追问、校勘层（M3 以后）
- 多古籍切换 UI、模板缓存、gzip、TLS
