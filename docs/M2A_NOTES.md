# M2a · zgc-core（UI 无关的内核）实现说明与实测

> 目标：只做与渲染无关的 Rust 内核，任何 UI（Leptos / egui / …）都能直接复用。
> 本 crate **不含任何 UI 依赖**，依赖只有 `serde` + `serde_json`。

## 1. 交付物与命令

    Cargo.toml                      workspace 根（members = crates/zgc-core）
    Cargo.lock                      依赖锁定
    crates/zgc-core/src/types.rs    schema 结构体（serde，camelCase）
    crates/zgc-core/src/load.rs     Book::load：读盘 + 文件集合校验
    crates/zgc-core/src/data.rs     BookData：不变量校验、索引、有序遍历
    crates/zgc-core/src/nav.rs      段/章/卷三级 prev-next
    crates/zgc-core/src/search.rs   CJK bigram 倒排索引 + 命中
    crates/zgc-core/src/error.rs    fail-fast 错误（带卷/章/段定位）
    crates/zgc-core/examples/probe.rs  真实数据探针
    crates/zgc-core/tests/load_fixtures.rs   加载器契约测试（临时夹具目录）
    crates/zgc-core/tests/data_integration.rs 真实 data/ 集成测试

运行：

    cargo test                                    # 全部测试（52 个）
    cargo run --release --example probe           # 真实数据探针（计数/耗时/检索/跳转）
    ZGC_DATA_DIR=/path/to/book cargo run --example probe   # 换数据目录

## 2. 模块与要点

- **类型层**（`types.rs`）：`Book / Group / Volume / Chapter / Paragraph / Annotation / Glossary`
  逐字段对应 `docs/SCHEMA.md`，统一 `#[serde(rename_all = "camelCase")]`。
  `annotations`/`glossary` 缺省为空 `Vec`；`translation`/`titleSimp`/`compiler`/`era`/
  `pinyin`/`gloss`/`source` 为 `Option`；`schemaVersion` 缺省 1。
  `form` 通过 `StructureAxis::from_form` 决定结构轴（國別體 → Polity；編年體 → Chronology，
  预留；未知/缺失 → Polity，不 panic）。
- **加载层**（`load.rs` + `data.rs`）：`Book::load(dir)` 只按 `volumeOrder` 读
  `volumes/<id>.json`（文件名与 `index` 都不参与排序）；随后由 `BookData::from_parts`
  统一校验并把卷按 `volumeOrder` 归一化（`from_parts` 也供编程构造/测试使用）。
- **导航层**（`nav.rs`）：`Position { volume, chapter, paragraph }` 为结构内导航句柄，
  公开锚点始终是段 id。空章、空卷会被跳过，不会出现“dead stop”。
- **检索层**（`search.rs`）：相邻二字（bigram）倒排，无分词器；查询用“最稀有 bigram”
  驱动候选，再用原文逐字复核，返回段级 id + 字段来源 + char/byte 偏移 + 上下文片段。

## 3. fail-fast 校验清单（全部有测试覆盖）

| 校验 | 错误 | 报错内容 |
|---|---|---|
| 段 id 全局唯一 | `DuplicateParagraphId` | 段 id + 先出现位置 + 再出现位置 |
| 章 id 全局唯一 | `DuplicateChapterId` | 章 id + 两处位置 |
| 卷文件集合 == volumeOrder | `MissingVolumeFile` / `UnexpectedVolumeFile` / `VolumeOrderMismatch` | 卷 id + 文件路径 |
| 卷文件内 `id` 与文件名一致 | `VolumeIdMismatch` | 期望 / 实际 / 路径 |
| `volumeOrder` 无重复、无路径分隔符 | `DuplicateVolumeOrderId` / `InvalidVolumeId` | 卷 id |
| group 引用的卷都存在 | `GroupReferencesUnknownVolume` | group id + 卷 id |
| 卷的 `groupId` 存在 | `VolumeReferencesUnknownGroup` | 卷 id + group id |
| 每卷至多属于 1 个 group（>1 即失败） | `VolumeInMultipleGroups` | 卷 id + 全部 group id |
| `text` 非空（含纯空白） | `EmptyParagraphText` | 卷 / 章 / 段 |
| `volumeOrder` 非空 | `EmptyVolumeOrder` | book.json 路径 |
| JSON/IO 错误 | `Json` / `Io` | 出错文件路径 |

错误 Display 示例（测试里断言过）：

    段 id dup-p0001 重复：先出现于 卷 a-vol / 章 a-vol-01，又出现于 卷 b-vol / 章 b-vol-01
    卷 a-vol / 章 a-vol-01 / 段 a-vol-01-p0001 的 text 为空
    group a 引用了不存在的卷 ghost-9（volumeOrder 中无此卷）
    卷 a-vol 被多个 group 列出：a, b（每卷必须恰好归属一个 group）

## 4. 导航语义（明确写死，UI 不用猜）

| 方法 | 行为 |
|---|---|
| `next_paragraph` / `prev_paragraph` | 段级前进/后退；跨章跨卷；后退跨界时落到**上一章末段** |
| `next_chapter` / `prev_chapter` | 章级跳转；跨卷；落到目标章**首段** |
| `next_volume` / `prev_volume` | 卷级跳转；落到目标卷**首段** |
| `iter_positions` / `iter_paragraphs` | 全书阅读顺序（volumeOrder → 章序 → 段序） |
| `group_volumes(id)` / `group_volume_indices(id)` | 国 → 卷（按 volumeOrder，不按 group.volumes 书写顺序） |

## 5. 检索设计

- 索引字段：`text`（繁体原文）、`translation`（白话）、`annotations[].text`、`glossary[].term`。
- 键：相邻两个字（`Bigram(u64)` 打包），空白相邻对不入索引（避免跨词噪声）。
- 值：`Posting { 段扁平序号, 字段来源, 字段内序号, char 偏移, byte 偏移 }`。
- 查询：查询串取相邻二字；选**倒排表最短**的 bigram 当驱动（并列时取靠前的相对位置，
  保证确定性），用 `char_offset - 相对位置` 求候选起点，再从该字节位置**逐字复核**整串；
  单字/纯空白查询退化为线性扫描（不 panic，结果同样稳定）。
- 输出：`Hit { volume_id, chapter_id, paragraph_id, field, field_index, char_offset,
  byte_offset, snippet }`；`char_offset` 供 UI 光标，`byte_offset` 是可直接切片的字符边界。
- 排序：`(卷序, 章序, 段序, 字段 Text<Translation<Annotation<Glossary, 字段内序号, 偏移)`，
  排序后去重 ⇒ 多次查询、多次建索引结果完全相同（集成测试断言）。
- 范围：`Scope::All | Scope::Group(id) | Scope::Volume(id)`，用卷位图过滤；未知范围返回空。
- 复杂度：建索引 O(总字符数)；查询 O(驱动 bigram 的 posting 数 × 命中长度)，不含分词、无外部词典。

## 6. 实测数据（release，本机 rustc 1.99.1 / macOS）

    冷启动首轮：load 11.973 ms / index 23.551 ms
    稳态三轮：  load 5.46 / 5.46 / 5.61 ms
               index 15.88 / 15.14 / 15.32 ms

建索引规模：**91152 个 distinct bigram / 370660 次 bigram 出现**（全书 4 个字段）。
（debug 构建：load 29.193 ms / index 126.605 ms，仅作参考；以上均为 release + 页面缓存命中。）

计数与验收目标一致：**33 卷 / 11 组 / 497 章 / 6663 段**（宋衛为一卷，通行本 11 国策；
集成测试硬断言，不是打印后目测）。

`cargo build --release`：**0 warning**（`cargo build --release 2>&1 | grep -ci warning` → 0）。

`cargo test` 结果：

    running 32 tests   (src 单元测试 + doctest 之外)
    test result: ok. 32 passed; 0 failed
    running 7 tests    (真实 data/ 集成测试)
    test result: ok. 7 passed; 0 failed
    running 12 tests   (加载器契约/夹具测试，含編年體不崩 + 一卷两组用例)
    test result: ok. 12 passed; 0 failed
    running 1 test     (doctest，compile)
    test result: ok. 1 passed; 0 failed

## 7. 真实检索样例（可逐字复核）

探针原始输出（`cargo run --release --example probe`）：

    query “衞鞅” 字段=text 命中 5 处（出现次数）
      - qin-1/qin-1-01/qin-1-01-p0001 [text] char=0 byte=0  衞鞅亡魏入秦，孝公以爲相…
      - qi-5/qi-5-01/qi-5-01-p0083 [text] char=0 byte=0  衞鞅謀于秦王曰：『夫魏氏…
      - qi-5/qi-5-01/qi-5-01-p0085 [text] char=5 byte=15  秦王許諾。衞鞅見魏王曰：『大王之功…

    query “蘇秦” 字段=text 命中 92 处
      - dongzhou/dongzhou-18/dongzhou-18-p0001 [text] char=6 byte=18  蘇厲爲周最謂蘇秦曰：「君不如令王聽最…
      - xizhou/xizhou-08/xizhou-08-p0002 [text] char=0 byte=0  蘇秦謂周君曰：「除道屬之…
      - qin-1/qin-1-02/qin-1-02-p0001 [text] char=0 byte=0  蘇秦始將連橫說秦惠王曰：…

    query “秦國” 字段=text 命中 10 处
      - qin-1/qin-1-05/qin-1-05-p0054 [text] char=19 byte=57  …數千里，各師數百萬，秦國號令賞罰，地形利害，…
      - qin-2/qin-2-05/qin-2-05-p0002 [text] char=34 byte=102  …與不知者敗之。使此知秦國之政也，則君一舉而亡…
      - qin-3/qin-3-09/qin-3-09-p0032 [text] char=14 byte=42  …：「先生是何言也！夫秦國僻遠，寡人愚不肖，先…

    query “秦国” 字段=translation 命中 1651 处
      - dongzhou/dongzhou-01/dongzhou-01-p0001 [translation] char=0 byte=0  秦国发动军队逼近洛阳城下…
      - dongzhou/dongzhou-01/dongzhou-01-p0003 [translation] char=13 byte=37  …到齐国，对齐王说： 秦国的这种行为残暴无道，…
      - dongzhou/dongzhou-01/dongzhou-01-p0003 [translation] char=56 byte=166  …策，认为与其将九鼎给秦国，不如将它送给贵国。

独立复核（绕开 Rust，直接读 data/ 的 python 脚本）逐条命中：

    qin-1-01-p0001 text offset=0 slice='衞鞅' OK=True
    qi-5-01-p0083 text offset=0 slice='衞鞅' OK=True
    qi-5-01-p0085 text offset=5 slice='衞鞅' OK=True
    dongzhou-01-p0003 translation offset=13 slice='秦国' OK=True
    dongzhou-01-p0003 translation offset=56 slice='秦国' OK=True

注意：**底本用「衞鞅」不是「衛鞅」**（数据里 `衛鞅` 为 0 命中，`衞鞅` 5 处）——这正是
“检索必须对原文逐字校验”的反例价值。命中数是**出现次数**（同段重复出现算多条），
段数会略小：蘇秦 92 处 / 81 段，秦国 1651 处 / 1248 段，秦王 241 处 / 227 段。

范围检索：`group qin`（5 卷）「秦王」164 处、`group qi`（6 卷）30 处，全部落在本组卷内（断言为 true）。

## 8. 跨章 / 跨卷跳转证据

    qin-1 末章末段: qin-1/qin-1-13/qin-1-13-p0025
      next_paragraph → qin-2/qin-2-01/qin-2-01-p0001（跨卷）
      next_volume    → qin-2/qin-2-01/qin-2-01-p0001（跨卷，对齐首段）
    qin-2 首章首段: qin-2/qin-2-01/qin-2-01-p0001
      prev_paragraph → qin-1/qin-1-13/qin-1-13-p0025（跨卷，落到上一卷末段）
      prev_volume    → qin-1/qin-1-01/qin-1-01-p0001（跨卷，对齐首段）
    qin-1 首章首段: qin-1/qin-1-01/qin-1-01-p0001
      next_chapter   → qin-1/qin-1-02/qin-1-02-p0001（跨章，对齐首段）
      prev_chapter   → xizhou/xizhou-17/xizhou-17-p0001（跨卷，对齐首段）

集成测试 `navigation_crosses_chapter_and_volume_boundaries_on_real_data` 把上述 id 硬断言；
另有夹具测试覆盖「章内跨章」「空章跳过」「卷尾跨卷」等边界，以及全书顺序遍历
（`walked == paragraph_count`，且 `first` 无 prev、`last` 无 next）。

## 9. 依赖清单与理由

直接依赖（`[dependencies]`）：

| 依赖 | 版本（解析） | 理由 |
|---|---|---|
| `serde` | 1.0.229（features = derive） | 结构体反序列化/序列化，schema 字段映射 |
| `serde_json` | 1.0.154 | 读 book.json / volumes/*.json |

传递依赖（`cargo tree`，全部由上面二者引入，无额外功能依赖）：

    serde_core, serde_derive, proc-macro2, quote, syn, unicode-ident   (derive 展开)
    itoa, memchr, zmij                                                (serde_json)

**没有** leptos / egui / dioxus / yew / tantivy / jieba / unicode-segmentation。
中文字符按 `char` 处理，bigram 不需要分词器，因此 `unicode-segmentation` 也不需要。

## 10. 假设与实现决策（用户未逐条确认，按此推进）

1. `group.volumes` 只用于“哪些卷属于这一组”，组内顺序仍以 `volumeOrder` 为准；
   组内重复列出的卷只计一次（`sort + dedup`）。
2. 卷的 `groupId` 为空字符串时视为“未分组”，不报错；非空则必须在 `groups` 中存在。
3. `prev_chapter` / `prev_volume` 对齐到目标**首段**（阅读器「上一章」通常回到章首）；
   段级后退才落到上一章末段。
4. `annotations` / `glossary` 当前全为空（M1b 正在写入）：索引层天然支持空集合，
   集成测试对“空”与“非空”都成立（若注疏已写入，会逐条校验其可检索；当前实测
   `annotation checks: 0, glossary checks: 0`）。
5. 检索大小为「出现次数」而非「段数」；UI 若需去重列表可自行按 `paragraph_id` 聚合。
6. ASCII 大小写折叠（中文不受影响），空白字符不参与 bigram；查询串会先 trim。

## 11. 留给 UI / 后续（本任务明确不做）

- `form = 編年體` 只保证“能加载、能遍历、不 panic”，真正的帝王→年导航轴留给后续古籍。
- 无持久化（进度记忆、批注落盘）、无分页/分栏渲染、无 AI 调用——都在 UI 层或后续里程碑。
- 单字/极短查询走线性扫描（6663 段量级下毫秒级，见样例 0.01–0.02 ms；超长语料再考虑加
  unigram 索引）。
