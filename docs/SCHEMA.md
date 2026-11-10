# 数据模型规范 v1（canonical schema）

**这份 schema 是应用唯一读取契约。** 抓取端源格式各异，一律归一化到此结构。

## book.json

```json
{
  "schemaVersion": 1,
  "id": "zhanguoce",
  "title": "戰國策",
  "titleSimp": "战国策",
  "compiler": "劉向",
  "era": "西漢",
  "form": "國別體",
  "sources": [{ "name": "...", "url": "...", "license": "public-domain" }],
  "groups": [
    { "id": "qin", "title": "秦策", "titleSimp": "秦策", "volumes": ["qin-1", "qin-2"] }
  ],
  "volumeOrder": ["dongzhou", "xizhou", "qin-1", "qin-2"]
}
```

- form 决定 UI 导航轴：國別體 → 国→卷；編年體 → 帝王→年。这是「加数据不加代码」的关键字段。
- volumeOrder 为全书阅读顺序，UI 不得自行排序。

## volumes/<vol-id>.json

```json
{
  "schemaVersion": 1,
  "id": "qin-1",
  "bookId": "zhanguoce",
  "groupId": "qin",
  "title": "秦策一",
  "titleSimp": "秦策一",
  "index": 3,
  "chapters": [
    {
      "id": "qin-1-01",
      "title": "衛鞅亡魏入秦",
      "paragraphs": [
        {
          "id": "qin-1-01-p0001",
          "text": "衛鞅亡魏入秦，孝公以為相，封之於商，號曰商君。",
          "translation": "卫鞅从魏国逃到秦国，秦孝公任命他为相……",
          "annotations": [{ "source": "鮑彪", "text": "……", "edition": "四部叢刊" }],
          "glossary": [{ "term": "衛鞅", "pinyin": "wei yang", "gloss": "即商鞅，法家代表人物" }]
        }
      ]
    }
  ]
}
```

## 硬性约束

1. **id 稳定且全局唯一**：<vol-id>-<章序2位>-p<段序4位>。段 id 是全系统锚点，重复运行必须产出相同 id（幂等）。
2. text 用**繁体**，translation 用**简体**。不得倒置。
3. annotations 可为空数组；glossary 可为空数组。**不得**用空字符串占位。
4. 缺失数据一律 null 或空数组，**禁止伪造内容**。凡不确定处写进 docs/DATA_REPORT.md 的存疑清单。
5. 无 BOM、UTF-8、LF 换行、2 空格缩进。
6. 段内不得含 HTML 标签；原文标点保留底本原貌。
7. annotations 条目字段：`source` ∈ {`鲍彪` / `吴师道补` / `吴师道正`}（作者）；
   可选 `edition` ∈ {`四部叢刊` / `四庫全書`}（見證本）。`edition` 缺省即 `四部叢刊`，
   構建器對底本注不寫該鍵（保持 8006 條注的字節穩定），只對四庫全書本補入的注顯式寫
   `四庫全書`。作者不因版本而變：同一條鮑彪注在兩個本子裡都是 `鲍彪`，差別在見證本。
8. 同一段**不得混排兩個見證本**：四庫全書本只補「該段原本零注」的位置，
   既有注段落一律不動。
