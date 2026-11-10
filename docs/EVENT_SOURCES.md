# 《战国策》事件系年与互见对勘：数据源实测报告

> 定位：`docs/PLAN.md` 第五节 M3（知识层：人物/地名/事件卡片与互见）之 **M3c 事件表**的前置调研，
> 为 M3a（`docs/ENTITIES.md`：人物/地名）之外的「史事层」选源。
>
> 方法：全部结论来自本机真实 HTTP 请求（curl，`--max-time <= 20`），逐条记录状态码、字节数、content-type 与逐字样本。
> 凡未实测者一律标注「未验证」，不臆测。实测日期 2026-11-05。

---

## 0. 一页结论

### 0.1 排序表

排序依据：①能否直接产出「公元／王年」而不靠外部换算 → ②结构是否机器可读且稳定 → ③是否覆盖战国全段 → ④授权可用性。

| # | 来源 | 内容 | 获取方式 | 实测状态码 | 样本摘录 | 版权状态 | 结论 |
|---|---|---|---|---|---|---|---|
| 1 | **Ancient-China-Books/shiji**（`OEBPS/Text/014.htm` 十二諸侯年表、`015.htm` 六國年表） | 史记（年表＝**系年轴**） | raw → `https://raw.githubusercontent.com/Ancient-China-Books/shiji/master/OEBPS/Text/015.htm`；raw 不通时走 GitHub contents API（见 §5） | **200 / 139978 B**（015）、**200 / 228835 B**（014） | `<tr><td>公元前<span class="num">476</span></td><td>元王元年<p class="style7" id="p6">〔集解〕徐廣曰：「乙丑」…` | README：「基於漢川草廬重製」；**无 LICENSE（404 / 14 B）** | **必用**。014＝841–477 BC 共 **365 行**（含干支列）；015＝476–207 BC 共 **270 行**；两卷相接、无缺口 → 841–207 BC 无缝年轴 |
| 2 | **Ancient-China-Books/zizhitongjian**（`5282/5381/5382/5383/5384/5385/5386.html`） | 通鉴（**自带绝对纪年＋段锚点**） | `.../zizhitongjian/master/OEBPS/Text/5282.html` | **200 / 120938 B**（5282）、**200 / 119345 B**（5381） | `<div class="yt" id="y0403"><p class="strong" id="p3">…<span data-head-title="二十三年">二十三年</span><span class="biaozhusup" data-rel="">（戊寅、前四○三年）</span>` | README：「基於東里書齋重製」；**LICENSE 404** | **必用**。`div.yt#yNNNN`＝绝对公元前年、`p#pN`＝段锚点、同 div 内带王年与干支 → **唯一无需换算即可对齐公元的正文源** |
| 3 | **殆知阁镜像 `garychowcmu/daizhigev20` → `史藏/编年/大事记.txt`**（吕祖谦《大事记》） | 战国策／史记／通鉴（**事件层、「战国策」出处标注**） | GitHub contents API（raw 当前不可达，见 §5） | **200 / 1386446 B**（base64 体），解码 **338605 字** | `周威烈王二十三年【魏文侯斯二十二年韩景侯防六年赵烈侯籍六年楚声王五年燕闵公三十一年宋悼公购由元年】` ／ `晋荀瑶袭卫【以古史战国策修】` | 民间整理四库本，repo 无 LICENSE（未逐字核验） | **最有价值的「桥」**。每条＝周王年＋【并列各国君在位年】，408 个周王年条目，事件逐条注「以某书修」，**51 处直接标 战国策/国策/战国防**；覆盖 前 501–前 90，战国全段不缺。**无公元列**（「公元前」0 次），须配 #1 换算 |
| 4 | **kanripo `KR2a0001`＝史記**（`_202.txt` 十二諸侯年表、`_203.txt` 六國年表） | 史记（年表，冗余校验） | `https://raw.githubusercontent.com/kanripo/KR2a0001/master/KR2a0001_203.txt` | **200 / 42409 B**（203）、**200 / 74256 B**（202） | `公元前|476元王元年|厲共公元年|衞出公輒後元年。||¶` | kanripo 依 SBCK/TLS，学术用途 | **可用作 #1 的对照版**（管道分隔更好解析，但无结构）。注：`# dating: 6220` 全文件 14 处同值 → **逐段系年价值为零** |
| 5 | **kanripo `KR2b0007`＝資治通鑑** ／ `KR2c0001`＝通鑑紀事本末 | 通鉴（胡注本／事件本体） | `raw.../KR2b0007/master/KR2b0007_001.txt` | **200 / 62776 B**（001，24199 字） | `　周紀一(起著雍攝提格盡𤣥黓困敦凡三十五年丙/…` ／ `　威烈王(名午考王之子諡法…` | 同上 | **次选**。有王年、**无公元、无干支**，胡注与正文括号交织，清洗成本高；`KR2c0001` 未深入采样（**未验证**） |
| 6 | **Ancient-China-Books/zhanguoce** | 战国策正文（繁体白文，497 章） | `.../zhanguoce/master/OEBPS/Text/001.htm` | **200 / 2294 B**（001，1204 字）；树 200 / 154720 B | `<h5>戰國策</h5><span class="style9">卷一 ‧ 東周策</span><hr/><p class="style6">秦興師臨周而求九鼎</p>` | 同 #1，无 LICENSE | **只作比对文本**。一章一文件、**无段落 id、无注、无年** |
| 7 | **kanripo `KR2e0003`＝戰國策校注**（鮑彪注＋吳師道補正） | 战国策注（**章→王年 的注文线索**） | `raw.../KR2e0003/master/KR2e0003_001.txt` | **200 / 41100 B**（001，16347 字） | `赧王(周紀慎靚王子歲丁未立徙都西周…` ／ `(正曰此據史按通鑑大事記顯王三十五年乃魏惠王後元年…)` | 同上 | **必用的注文源**。注文以周王年分组（赧王 10 次、烈王 5 次、顯王 2 次），正曰直接引「通鑑大事記」换算 → 部分章**可直接读出王年归属** |
| 8 | **dutongjian.com `/api/table_of_contents`** | 通鉴（帝王—年—段 索引，含 `year_int`） | `GET https://www.dutongjian.com/api/table_of_contents` | **200 / 336149 B**，`application/json`，0.42 s | `\"emperor_name\":\"威烈王\"…\"reign_name\":\"二十三年\",\"year_anno\":\"（戊寅、前四○三）\",\"year_int\":-403` | **第三方商业产品（《读通鉴》）**，无授权声明 | **仅作交叉校验**。294 卷全覆盖、每年有 `year_int`；但无正文、授权不明 |
| 9 | **CBDB（哈佛）** | 人物生卒／官职 | `https://cbdb.fas.harvard.edu/cbdbapi/person.php?id=1024` | **200 / 40976 B** `text/html`；`/cbdbapi/` → **302**；`query.php` → **200 / 0 B** | 页面仅 `CBDB 人物資料庫 - 1024` / `載入中…`（JS 壳，无内联数据、无 API 目标） | 许可页 → **403 / 418 B**（Akamai） | **未验证／不作主路**。站点可达但**无可用机器接口证据** |
| 10 | 殆知阁 `史藏/正史/史记.txt`、`史藏/编年/资治通鉴.txt` | 史记／通鉴（简体全文，**年表已损**） | 同 #3 通道 | **200 / 1784829 B**（史记）、**200 / 9588649 B**（通鉴，3216660 字） | 史记：`太史公读秦记，至犬戎败幽王…`（年表被拍平为散文，**无年份行**）；通鉴：`　　威烈王二十三年` / `　　初命晋大夫魏斯、赵籍、韩虔为诸侯。` | 同上 | **不适合建系年轴**。通鉴有王年行但**无公元、无干支** |
| 11 | 殆知阁 `资治通鉴目录.txt` ／ kanripo `KR2b0010`＝資治通鑑目錄 | 司马光「年经国纬」事件索引 | 两处均实取 | **200 / 36217 B**（15265 字）／**200 / 2181 B**（1917 字） | 仅 `資治通鑑目録卷一 宋 司馬光 撰` ＋ 大量 `<史部,编年类,资治通鉴目录,卷一>` 占位符 | — | **空壳，此捷径不存在**（两源均已实测） |
| 12 | 书格 shuge.org | 影像 | `https://www.shuge.org/` | **200 / 138369 B** | — | 影像站 | **无机器可读正文**，仅供校勘对照版面 |
| 13 | GitHub 仓库搜索 | 找结构化数据 | `/search/repositories?q=资治通鉴 json` | **200，total 0** | — | — | **无现成结构化通鉴仓库**；`q=通鑑 org:kanripo` → total 28（据此可列全 KR 编号） |

### 0.2 最省的一条路（四层管道）

**核心洞察：《战国策》自己不系年，但「谁给它系年」有明确的历史答案——鮑彪/吳師道注里的换算依据就写着 `通鑑大事記`；而《大事记》（吕祖谦）恰是逐条标注「以战国策修」的编年史。** 因此不必做「战国策 ↔ 通鉴」的盲目相似度匹配，可以沿 注文 → 大事记 → 年表 这条**有出处的链**走。

```
第 1 层  年轴（王年 ↔ 公元）
        shiji/014.htm（841–477，365 行，含干支）+ shiji/015.htm（476–207，270 行）
        → 构造 (周王年, 某国某君某年) → 公元前 的查表；两卷相接无缺口。

第 2 层  事件层（自带公元 + 段锚点）
        zizhitongjian/5282,5381,5382,5383,5384,5385,5386.html
        → 解析 div.yt#yNNNN（公元前）+ p#pN（段锚点）+ data-head-title（王年）+ biaozhusup（干支）
        → 直接得 event{year_bc, ruler_year, ganzhi, anchor}，无需换算。

第 3 层  桥（战国策章 → 王年）
        (a) KR2e0003 注文：赧王/安王/烈王/顯王/慎靚王 分组 + 「通鑑大事記…當…」换算句 → 部分章直接得王年；
        (b) daizhige 大事记.txt：408 个周王年条目，事件后标「以古史战国策修」「以通鉴战国防修」等
            （全文 51 处提及 战国策/国策）→ 事件句与战国策章正文匹配，命中章继承周王年；
        (c) 仍未命中的章，再用 5282…5386 正文做兜底匹配（相似度最弱，须抽检）。

第 4 层  人物生卒补 summary
        CBDB 现无机器接口 → 暂缓；先用 史记 世家/列传 与 大事记 的君年信息补。
```

**成本**：第 1、2 层纯解析、无需人工判断；第 3 层 (a) 是读注文，(b) 最有效但需简繁归一＋四库讹字归并（该本常把「策」写作「防」）。

### 0.3 这条路缺什么（M3c 必须写进计划的风险）

1. **不存在公开的「战国策 497 章 → 年」逐章对照表**。已排除：GitHub 仓库搜索 `资治通鉴 json` total 0；通鑑目錄两源皆空壳。**必须自建。**
2. **《大事记》无公元列**（「公元前」0 次）且为**简体＋四库本讹字**（「策」→「防」），需归一化后再经第 1 层换算。
3. **通鑑不逐年**：`5282.html` 覆盖 前403–前369（卷首「凡三十五年」）却只有 **30 个 `div.yt`**，缺 374/384/388/392/395；`5381.html` 覆盖 前368–前321 只有 **40 个**，缺 331/339/342/345/349/360/363/367 → **逐年空轴必须由第 1 层年表补**。
4. **通鑑是选录本**：战国策大量章（东周/西周/中山/宋卫等）在通鑑无对应段，兜底匹配会系统性漏检。
5. **战国策章节本身可能跨年**（一篇多事、事后追述），「单章单年」假设会错 → 事件表应以**段**为单位。
6. **人物生卒空缺**：CBDB 无可用的机器接口（§1.7），`summary` 中的生卒建议按 `docs/ENTITIES.md` 契约置 `null`。
7. **授权不明**：两个主力仓库**均无 LICENSE（404）**，仅 README 称「重制自 漢川草廬／東里書齋」。古籍原文属公有领域，但整理本的标点/校勘/排版可能主张权利 → 建议内部研究使用；对外发布时只发布自建索引与指向原站链接，不分发整理本正文。

---

## 1. 逐源实测明细

### 1.1 Ancient-China-Books/shiji —— 系年轴首选

```
GET https://api.github.com/orgs/Ancient-China-Books/repos?per_page=100  -> 200（46 个 repo）
GET .../shiji/master/README.md   -> 200 / 104 B
    全文：# 史記三家注  基於「[漢川草廬](http://www.sidneyluo.net/a/a01/a01.htm)」重製電子版。
GET .../shiji/master/LICENSE     -> 404 / 14 B（404: Not Found）
GET .../shiji/git/trees/master?recursive=1 -> 200 / 47361 B，161 路径，truncated=false
```

布局：`OEBPS/Text/001.htm`(97924 B) … `014.htm`(228835 B)、`015.htm`(139978 B) —— **一卷一文件**，无目录页。

卷十五 六國年表（`015.htm`）逐字结构：

```html
<tr class="style4"> <td> </td> <td class="tdT1">周</td> <td class="tdT1">秦</td>
  <td class="tdT1">魏獻子</td> <td class="tdT1">韓宣子</td> <td class="tdT1">趙簡子</td>
  <td class="tdT1">楚</td> <td class="tdT1">燕</td> <td class="tdT1">齊</td> </tr>
<tr> <td>公元前<span class="num">476</span></td> <td>元王元年<p class="style7" id="p6">…
  〔集解〕徐廣曰：「乙丑」。皇甫謐曰：「元年癸酉，二十八年庚子崩。」</p>…</td> …</tr>
<tr> <td><span class="num">475</span></td> <td>二</td> <td>二<p id="p15">蜀人來賂。</p></td> …</tr>
```

⚠️ **只有首行带「公元前」三字**，其后各行年份是纯 `<td><span class="num">475</span></td>` —— 解析时不得假设每行都有「公元前」。

实测统计（本地解析）：

| 文件 | `<tr>` 数 | 年份行数 | 区间 | 缺口 | 列 |
|---|---|---|---|---|---|
| `014.htm` 十二諸侯年表 | 366 | **365** | 841 BC → 477 BC | **无** | 公元前 / 年（**干支**）/ 周·魯·齊·晉·秦·楚·宋·衞·陳·蔡·曹·鄭·燕·吳 |
| `015.htm` 六國年表 | 271 | **270** | 476 BC → 207 BC | **无** | 公元前 / 周·秦·魏·韓·趙·楚·燕·齊 |

表头逐字（014）：`<tr class="style4"> <td>公元前</td> <td>年</td> <td class="tdT1">周</td> <td class="tdT1">魯</td> …` —— 第二列即**干支**，首行 `841 / 庚申 / 共和元年`。

→ **841–207 BC 连续 636 年、每年一行、每行含当年各国君位年**，这是把任意「某王／某君第 N 年」换算为公元的充分依据。

### 1.2 Ancient-China-Books/zizhitongjian —— 事件层首选

```
GET .../zizhitongjian/master/README.md -> 200 / 121 B
    全文：# 資治通鑑胡三省注  基於「[東里書齋](https://www.donglishuzhai.net/books/104.html)」重製電子版。
GET .../LICENSE                        -> 404 / 14 B
GET .../git/trees/master?recursive=1   -> 200 / 100958 B，332 路径
GET .../OEBPS/content.opf              -> 200 / 36605 B，315 items（spine 用 idref=id0…，文件名不按卷序）
```

战国段正文只需 **7 个文件**（按 `<h1>` 实测映射，不可按文件名排序）：

| 文件 | 卷 | 覆盖 | `div.yt` 数 | 实测 |
|---|---|---|---|---|
| `5282.html` | 周紀一 | 前403 – 前369 | 30 | 200 / **120938 B**，72615 字；王：威烈王、安王、烈王 |
| `5381.html` | 周紀二 | 前368 – 前321 | 40 | 200 / **119345 B**，72080 字；王：顯王 |
| `5382.html` | 周紀三 | 前320 – 前298 | 23 | 200 / **105133 B**，60811 字；王：愼靚王、赧王上 |
| `5383.html` | 周紀四 | 前297 – 前273 | 25 | 200 / **108365 B**，61974 字；王：赧王中（其 `data-head-title` 实测为 `周紀四<`，尾随杂字符） |
| `5384.html` | 周紀五 | 前272 – 前256 | 16 | 200 / **90132 B**，49892 字；王：赧王下 |
| `5385.html` | 秦紀一 | 前255 – 前228 | 28 | 200 / **119519 B**，69607 字；王：昭襄王、孝文王、莊襄王、始皇帝上 |
| `5386.html` | 秦紀二 | 前227 – 前209 | 18 | 200 / **107002 B**，60294 字；王：始皇帝下、二世皇帝上 |

（`5281.html` = 序，13702 B，无年鉴。）

**合计**：7 个文件 **457273 字**、**180 个 `div.yt` 年条目**；覆盖 前403–前209 共 195 年 → **15 个年份没有任何条目**（通鑑只记「有事之年」，见 §0.3 第 3 条）。

核心结构逐字（`5282.html`）：

```html
<h1><span class="section" id="title-1"><span data-head-description="周紀一" data-head-level="h2" data-head-title="周紀一">周紀一</span></span></h1>
<p class="" id="p0">…<span class="jiazhu">起著雍攝提格<span class="biaozhusup" data-rel="">（戊寅）</span>，盡玄黓困敦<span class="biaozhusup" data-rel="">（壬子）</span>，凡三十五年。</span></p>
<h5><span class="section" id="title-2"><span data-head-description="" data-head-level="h3" data-head-title="威烈王">威烈王</span></span></h5>

<div class="yt" id="y0403"><p class="strong" id="p3"><a class="gg-link" href="#p3" …></a>
  <span class="section" id="title-3"><span data-head-description="威烈王" data-head-level="h4" data-head-title="二十三年">二十三年</span></span>
  <span class="biaozhusup" data-rel="">（戊寅、前四○三年）</span>
  <span class="jiazhu">上距春秋獲麟七十八年，距左傳趙襄子惎智伯事七十一年。…</span></p></div>

<p class="em2 strong" id="p4">…（事件正文）</p>
```

**可直接机器对齐的三件事**：

1. `div.yt` 的 `id="yNNNN"` ＝**绝对公元前年**（`y0403` → 前 403），无需换算；
2. 同一 div 内 `data-head-title` ＝该年**王年**（「二十三年」），`biaozhusup` ＝**干支＋公元前中文数字**（「戊寅、前四○三年」）→ 内部自校验；
3. `p id="pN"` 是**段锚点**（5282 共 147 个、5381 共 153 个），可与 `docs/ENTITIES.md` 的段 id 契约直接对接。

### 1.3 kanripo —— KR 编号已定位（史記 `KR2a0001`、資治通鑑 `KR2b0007`）

布局注意：**repo 根下是 `KRxxxxx_NNN.txt`，不是 `1/1a.txt`**。

```
GET raw.../kanripo/KR2e0003/master/1/1a.txt         -> 404 / 14 B（旧路径猜测错误）
GET raw.../kanripo/KR2e0003/master/KR2e0003_001.txt -> 200 / 41100 B（16347 字）
    头：#+TITLE: 戰國策 / #+PROPERTY: BASEEDITION SBCK / #+PROPERTY: FILE SB02n0044-001戰國策校注-卷第一.
GET raw.../kanripo/KR2a0001/master/KR2a0001_203.txt -> 200 / 42409 B（19887 字，六國年表）
GET raw.../kanripo/KR2a0001/master/KR2a0001_202.txt -> 200 / 74256 B（十二諸侯年表）
GET raw.../kanripo/KR2b0007/master/KR2b0007_001.txt -> 200 / 62776 B（24199 字）
GET raw.../kanripo/KR2b0010/master/KR2b0010_001.txt -> 200 / 2181 B（1917 字，空壳）
```

`KR2a0001_203.txt` 表体（管道分隔，**首列即公元前年**）：

```
周|秦|魏獻子|韓宣子|趙簡子|楚|燕|齊||
公元前|476元王元年|厲共公元年|衞出公輒後元年。||¶
475二|二 蜀人來賂。|¶
…
207…三 趙高反，二世自殺|¶
```

文件头带版本溯源：`# src: SHIJI 15.02 , ed. Zhonghuashuju p. 0685; Wang Liqi 477; tr. Watson 1993, p85`（基于中华书局点校本）；但 `# dating: 6220` 在 14 处**取值完全相同**，逐段系年无区分度，**不可当段级年份使用**。

`KR2a0001_202.txt` 表头：`年|周|魯|齊|晉|秦|楚|宋|衞|陳|蔡|曹|鄭|燕|吳||`，首行 `公元前|841庚申|共和元年。…`（**干支与公元同格**，可直接校 §1.1）。

**通鑑（kanripo 本）次选**：`KR2b0007_001.txt` 起句 `　周紀一(起著雍攝提格盡𤣥黓困敦凡三十五年丙/…`，王年行 `　威烈王(名午考王之子諡法…`。特点是**胡三省音注直接夹在括号里**，且**无公元列、无独立干支列** → 用此本必须自建换算（回到 §1.1）。

`KR2b0010`＝資治通鑑目錄：**整 repo 29 个文件合计仅 54335 字节**，`001.txt` 正文只有 `<pb:KR2b0010_WYG_001-1a>¶`、「欽定四庫全書¶ 資治通鑑目録卷一¶ …宋 司馬光 撰」与连续空页 → **司马光「年经国纬」索引在此源不可用**。

### 1.4 殆知阁镜像：关键发现——《大事记》

仓 `garychowcmu/daizhigev20`（repo 页 200 / 231464 B），顶层 `佛藏/儒藏/医藏/史藏/子藏/易藏/艺藏/诗藏/道藏/集藏`。

`史藏/编年`（50 个文件）实测含：`资治通鉴.txt` 9588649、`资治通鉴注.txt` 17348890、`资治通鉴目录.txt` 36217、`资治通鉴考异.txt` 955299、`资治通鉴释例.txt` 6603、`通鉴地理通释.txt` 436825、`稽古录.txt`、**`大事记.txt` 1005493**、`竹书纪年.txt`。

**`大事记.txt`（吕祖谦）实测：200 / 1386446 B（base64），解码 338605 字。**

四库提要逐字：

```
　　【臣】等谨案大事记十二巻通释三巻解题十二巻宋吕祖谦撰取司马迁年表大事记之目编年系月以记春秋后事
复采左氏厯代史皇极经世通鉴稽古录辑而广之始自周敬王三十九年迄汉武帝征和三年…
每条之下各注从某书修云云以自附于述而不作之义
```

**条目格式（这正是「桥」）**：

```
　　周威烈王二十三年【魏文侯斯二十二年韩景侯防六年赵烈侯籍六年楚声王五年燕闵公三十一年宋悼公购由元年】
　　九鼎震【夲纪年表】
　　初命晋大夫魏斯赵籍韩防为诸侯【通鉴】
　　燕闵公薨子厘公立【以世家修】
　　楚声王围宋十月罢【以吕氏春秋修】
```

即 **周王年 ＋【并列各国君在位年】＋ 逐条事件 ＋【出处】**。

实测统计：

| 指标 | 值 |
|---|---|
| 全文长度 | 338605 字 |
| 周王年条目数（`周X王…年`） | **408** |
| 覆盖 | 周敬王三十九年 → 汉武帝征和三年（前 501 – 前 90） |
| 「公元前」出现次数 | **0**（无公元列，须配 §1.1 换算） |
| 提及 战国策/国策 | **51 处**（含 3 处「以战国策修」） |
| 王年分布 | 赧王 115、显王 95、安王 47、威烈王 45、贞定王 41、考王 21、敬王 14、元王 12、烈王 12、慎靓王 6 |

**直接标注战国策出处的实例（逐字）**：

```
甲申周贞定王十二年【晋出公十八年蔡声侯十五年】
　　晋荀瑶袭卫【以古史战国策修】
…
　　周赧王三十年【秦昭王二十二年赵惠文王十四年齐湣王二十九年楚顷襄王十四年】齐湣王杀司马穰苴狐咺陈举【以通鉴战国防修】
```

⚠️ 该本**简体＋四库讹字**：「策」常作「防」（如「战国防」）→ 归一化必须含 `防→策`，只搜「战国策」会漏。

同仓另两个文本的实测结论（**不能替代 §1.1**）：

* `史藏/正史/史记.txt`（200 / 1784829 B，UTF-8 简体）：年表**被拍平成散文**——定位「六国年表第三」（偏移 97391）后只有表序「太史公读秦记，至犬戎败幽王…」，**没有任何年份行** → 结构信息丢失。
* `史藏/编年/资治通鉴.txt`（200 / 9588649 B，3216660 字，简体、无注）：起 `●卷第一` / `　　【周纪一】　起著雍摄提格，尽玄黓困敦，凡三十五年。` / `　　威烈王二十三年` / `　　初命晋大夫魏斯、赵籍、韩虔为诸侯。` → **有王年行、无公元、无干支**（胡注已删，正文反而更干净）。
* `史藏/编年/资治通鉴目录.txt`（200 / 36217 B，15265 字）：仅四库提要＋「资治通鉴目録卷一 宋 司马光 撰」＋大量 `<史部,编年类,资治通鉴目录,卷一>` 占位符 → **空壳**。

### 1.5 Ancient-China-Books/zhanguoce —— 只作比对

```
GET .../zhanguoce/git/trees/master?recursive=1 -> 200 / 154720 B，521 路径
GET .../zhanguoce/OEBPS/Text/001.htm          -> 200 / 2294 B（1204 字）
```

共 **497 个正文 `.htm`**（与 M1 的 497 章数目一致）。结构：

```html
<h5>戰國策</h5><span class="style9">卷一 ‧ 東周策</span><hr/>
<p class="style6">秦興師臨周而求九鼎</p>
<p>秦興師臨周而求九鼎，周君患之，以告顏率。…</p>
```

→ 一章一文件、繁体白文、**无 `id`、无注、无年**。可作「章标题 ↔ 正文」的干净对照，但**不提供段级锚点**（M1 语料已是段级，不必重复引入）。

### 1.6 dutongjian.com —— 有年份索引，授权不明

```
GET https://www.dutongjian.com/api/table_of_contents
    -> 200 / 336149 B，content-type: application/json，0.42 s
```

结构（实测）：

```json
{"data":{"book_name":"资治通鉴","juan_list":[…294…],"tongjian_id":"8bdaac15-…"},"message":"success","success":true}

"emperor_list":[{"emperor_name":"威烈王","reign_list":[
  {"reign_name":"二十三年","tongjian_id":"8be19ea3-…","year_anno":"（戊寅、前四○三）","year_int":-403},
  {"reign_name":"二十四年","year_anno":"（己卯、前四○二）","year_int":-402}]}]
```

* 294 卷全覆盖；`juan_list[4]` 起于「赧王下 四十三年（己丑、前二七二）」。
* **每个「年」都有 `year_int` 绝对公元前** → 形态上是现成的通鑑年索引。
* 缺：**无正文**（正文端点未验证）、uuid 体系与任何公开版本不对应、**他人商业产品无授权** → 只当交叉校验（例如核对 §1.2 的 `yNNNN` 与 `year_int` 是否一致），**不批量取正文**。

### 1.7 CBDB —— 站点可达，无可用机器接口（未验证）

```
GET https://cbdb.fas.harvard.edu/                                    -> 302 / 0 B
GET https://cbdb.fas.harvard.edu/cbdbapi/                            -> 302 / 0 B
GET .../cbdbapi/person.php?id=1024                                   -> 200 / 40976 B  text/html
GET .../cbdbapi/person.php?id=1024&format=json                       -> 200 / 40976 B  text/html（同一页）
GET .../cbdbapi/query.php?id=1024                                    -> 200 / 0 B
GET https://cbdb.hsites.harvard.edu/cbdb-data-licensing-terms        -> 403 / 418 B（Akamai: You don't have permission to access …）
```

`person.php?id=1024` 的 40976 B 是 **JS 渲染壳**：正文只有「CBDB 人物資料庫 - 1024」「中國歷代人物傳記資料庫」「載入中…」「© China Biographical Database…CBDB License」；**无内联数据**，grep `生`／`卒` 均 0 命中，可见端点只有 `/codes/ADDR_CODES?search=` 与 `?id=`，**找不到实际取数 API**。

结论：**人物生卒目前拿不到机器可读数据**；批量数据需注册授权（许可页本身 403，条款**未验证**）。M3c 的 `summary` 若需生卒，应按 `docs/ENTITIES.md` 契约置 `null`，不得编造。

### 1.8 其它实测与未验证项

| 目标 | 结果 |
|---|---|
| `api.github.com/rate_limit` | 200 / 424 B；未认证 core 60/h、search 10/min（探测中 remaining 39–57）；**raw 站点与 API 配额互不相干** |
| `chinese-poetry/chinese-poetry` 顶层 | 27 项（全唐诗/宋词/元曲/四书五经/楚辞/诗经/论语/蒙学…）**无史部** → 该生态不含史记/通鉴 |
| `/search/repositories?q=通鑑 org:kanripo` | 200，total 28：`KR2b0007 資治通鑑-宋-`、`KR2b0010 資治通鑑目錄-宋-`、`KR2b0008 考異`、`KR2c0001 通鑑紀事本末-宋-`、`KR2b0011 通鑑地理通釋-宋-` |
| `/search/repositories?q=资治通鉴 json` | 200，**total 0** → 无现成结构化数据 |
| `https://www.shuge.org/` | 200 / 138369 B（影像站） |
| `https://www.daizhige.org/`、`http://` | **301 / 167 B**（未跟随，落地页**未验证**） |
| `https://sinocal.sinica.edu.tw/`（兩千年中西曆轉換） | `curl (60) SSL certificate problem: certificate has expired`，**code=000**（干支↔公元换算急需，**未验证**） |
| `raw.githubusercontent.com` 稳定性 | **会成批失败**：连续请求后出现持续 `code=000` / `curl: (28) Connection timed out after 20005 milliseconds`（同刻 `api.github.com` 仍 200 / 0.23 s）→ **必须实现回退通道**，见 §5 |

---

## 2. 已实测的死源（用户给定，**本次未重测**）

以下沿用既有记录，本报告未重复验证，引用时保持「未重测」标注：

| 源 | 症状 |
|---|---|
| `ctext.org` 正文页 | Cloudflare Turnstile 挑战页（2781 字节） |
| ctext.org API | `ERR_REQUIRES_AUTHENTICATION` |
| `zh.wikisource.org` | HTTP 000（wikimedia 域本机不可达） |
| `archive.org` | HTTP 000 |
| `zhonghuadiancang.com` | HTTP 000 |
| `guoxuedashi.net` | 200，但正文仅 125 字节（反爬壳） |
| GitHub **代码**搜索 API | 401（需认证） |

补充（本次实测）：GitHub **仓库**搜索 API（`/search/repositories`）**无需认证可用**，与代码搜索不同，可据此按 KR 编号列目录（§1.8）。

---

## 3. 落地方案（若要做事件表）

### 3.1 建议的 `events.json` 最小字段

```json
{
  "event_id": "tj-5282-y0403-p3",
  "year_bc": 403,
  "year_src": "zizhitongjian:5282#y0403",
  "ganzhi": "戊寅",
  "ruler": "周威烈王",
  "ruler_year": 23,
  "state_years": {"魏": "魏文侯二十二年", "韓": "韓景侯六年"},
  "text": "初命晉大夫魏斯、趙籍、韓虔為諸侯。",
  "anchor": {"file": "OEBPS/Text/5282.html", "div": "y0403", "p": "p4"},
  "zgz_chapters": [],
  "crossrefs": [{"src": "大事记", "quote": "初命晋大夫魏斯赵籍韩防为诸侯【通鉴】", "year_bc": 403}],
  "confidence": "high|medium|low",
  "date_note": null
}
```

要点：**`year_bc` 只允许来自 §1.1／§1.2 的实测锚点**；`confidence` 反映匹配方式（注文直读 > 大事记条目命中 > 正文相似度），与 `docs/ENTITIES.md` 的「不许编造」一致。

### 3.2 实现顺序（每步可独立验收）

1. **年轴**：解析 `shiji/014.htm` + `015.htm` → `(year_bc, 干支, {国: 君+年})` 636 行表。验收：841／477／476／207 四个端点与首末行逐字对齐。
2. **事件层**：解析 `zizhitongjian/5282,5381–5386` → `event{year_bc, ruler_year, ganzhi, p_anchor, text}`。验收：`y0403` 在 `5282.html` 内且其 `biaozhusup` 含「前四○三年」；`div.yt` 总数＝各文件实测值（30/40/23/25/16/28/18，合计 **180**）。
3. **桥-A（注文）**：从 `KR2e0003_00N.txt` 抽 赧王/安王/烈王/顯王/慎靚王 分组与含「通鑑大事記」的换算句 → 部分章直接得王年。验收：每条可回指注文原文偏移。
4. **桥-B（大事记）**：解析 `大事记.txt` 的 408 个周王年条目与事件行，按 `防→策` 归一后与战国策匹配 → 章级候选年＋出处引文。验收：抽 10 条人工复核。
5. **兜底（相似度）**：仅对 3、4 未命中的章与通鑑战国段做 n-gram 匹配，`confidence=low`。

### 3.3 明确不要做的事

* 不要把 `史藏/正史/史记.txt` 当系年来源（年表已拍平，**无年份行**）。
* 不要指望通鑑目錄（kanripo `KR2b0010` 与殆知阁 `资治通鉴目录.txt` **两处都是空壳**）。
* 不要用 `KR2a0001` 的 `# dating:` 注释做段级年份（14 处同值 `6220`）。
* 不要批量抓 dutongjian.com 正文（他人产品、无授权）。

---

## 4. 缺口清单（按对 M3c 的影响排序）

1. **无「战国策章 → 年」公开对照表** —— 必须自建（桥-A + 桥-B + 兜底），最大工作量。
2. **《大事记》无公元列＋简体讹字** —— 需年表换算与 `防→策` 归一；四库本 OCR 质量未逐字核验（**未验证**）。
3. **通鑑只记「有事之年」** —— 5282 的 35 年区间仅 30 条（缺 374/384/388/392/395），5381 的 48 年区间仅 40 条（缺 331/339/342/345/349/360/363/367）→ 逐年刻度须由年轴补空。
4. **通鑑是选录本** —— 战国策大量章无对应段，兜底匹配会系统性漏检。
5. **人物生卒空缺** —— CBDB 无机器接口；`summary` 的生卒建议置 `null`，或只取 史记 世家/列传 明文「立年/卒年」。
6. **授权不明** —— Ancient-China-Books 两个主力仓**均无 LICENSE（404）**，仅 README 称重制自 漢川草廬／東里書齋，上游亦未声明许可。**古籍原文属公有领域，但整理本标点/校勘/排版可能主张权利** → 内部研究可用；对外发布只发自建索引与链接，不分发整理本正文。

---

## 5. 操作注意（可复现）

### 5.1 raw 不稳定 → 用 GitHub contents API 回退

实测：`raw.githubusercontent.com` 连续请求后会**成批超时**（`curl: (28) Connection timed out after 20005 milliseconds`，`code=000`），同刻 `api.github.com` 仍 200 / 0.23 s。回退命令（返回 base64，适用 < 1 MB 文件）：

```bash
curl -sS --max-time 20 \
  "https://api.github.com/repos/Ancient-China-Books/shiji/contents/OEBPS/Text/015.htm?ref=master" \
  -o /tmp/015.json
python3 - <<'PY'
import json, base64
d = json.load(open('/tmp/015.json'))
open('/tmp/015.htm', 'w').write(base64.b64decode(d['content']).decode('utf-8'))
print(d['name'], d['size'])   # -> 015.htm 139978
PY
```

该回退通道实测：`015.htm` → 200 / 193743 B / 0.75 s；`014.htm` → 200 / 316169 B / 0.85 s；`5282.html` → 200 / 167579 B / 0.72 s；`5381.html` → 200 / 165385 B / 1.09 s；`大事记.txt` → 200 / 1386446 B / 1.34 s。**均可用。**

### 5.2 其它

* 未认证配额 core 60/h、search 10/min；`contents` 调用计入 core。批量时缓存到本地（**不要把大语料写进 workspace**，探测产物放 `/tmp`，只把派生小表入库）。
* 所有请求加 `--max-time 20`；raw 需 3–4 次重试，**6 路以上并发会全部失败** → 串行＋重试。
* 解析 `015.htm`：年份只在**首行**带「公元前」三字，其余为纯 `<span class="num">475</span>`。

---

## 6. 附：本次实测命令清单（节选）

```bash
# 1) 组织与仓库
curl -sS --max-time 20 "https://api.github.com/orgs/Ancient-China-Books/repos?per_page=100" -o /tmp/org.json
# 2) 年表
curl -sS --max-time 20 "https://raw.githubusercontent.com/Ancient-China-Books/shiji/master/OEBPS/Text/015.htm" -o /tmp/sj/015.htm
# 3) 通鑑（自带 yNNNN）
curl -sS --max-time 20 "https://raw.githubusercontent.com/Ancient-China-Books/zizhitongjian/master/OEBPS/Text/5282.html" -o /tmp/tj/5282.html
# 4) 战国策校注（注文系年线索）
curl -sS --max-time 20 "https://raw.githubusercontent.com/kanripo/KR2e0003/master/KR2e0003_001.txt" -o /tmp/zgc.txt
# 5) 大事记（事件桥）
curl -sS --max-time 20 "https://api.github.com/repos/garychowcmu/daizhigev20/contents/%E5%8F%B2%E8%97%8F/%E7%BC%96%E5%B9%B4/%E5%A4%A7%E4%BA%8B%E8%AE%B0.txt" -o /tmp/dsj.json
# 6) 年份锚点计数（本地）
grep -o 'id="y[0-9]*"' /tmp/tj/5282.html | wc -l    # -> 30
grep -o '<tr>' /tmp/sj/015.htm | wc -l              # -> 271（含 1 表头）
```

---

*本报告所有状态码、字节数与引文均为本机实测；标「未验证」者为未取得可用证据，不做推断。*
