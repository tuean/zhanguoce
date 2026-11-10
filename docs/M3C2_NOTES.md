# M3c-2 · 戰國策章 ⟷ 年份 證據化系年

本輪把 497 章中的 **67 章** 判到具體年份，其餘 430 章進 unresolved 並給出固定枚舉的原因。
**覆蓋率 13.5%（67/497），精度自檢 30/30 = 100%。** 覆蓋率不設下限，精度是硬線：寧可判不出，不許掛錯。

- confidence：**high 41 · medium 26**（沒有 low；low 一律不落庫）
- 方法：event-text-match 61、name-window 6
- 證據 kind：substring 61、name-overlap 53、name-window 6、substring-range 1
- unresolved 原因分佈：no-tongjian-match 392、below-threshold 24、tongjian-bucket-year 8、manual-review-reject 6

## 1. 數據契約（data/zhanguoce/events/chapter_years.json）

~~~json
{
  "schemaVersion": 1,
  "bookId": "zhanguoce",
  "source": "…",
  "generatedBy": "tools/build_chapter_years.py",
  "thresholds": { "L_MIN": 8, "DF_MAX": 2, "MARGIN": 2, "HIGH_L": 14 },
  "chapters": [{
    "chapterId": "qin-1-02",
    "yearBc": 333,
    "yearRange": [333, 333],
    "confidence": "high",
    "method": "event-text-match",
    "evidence": [{ "eventId": "tj-y0333-p12", "kind": "substring",
                  "matched": "…", "score": 0.86, "length": 18, "df": 1 }],
    "reason": null
  }],
  "unresolved": [{ "chapterId": "dongzhou-01", "reason": "no-tongjian-match" }]
}
~~~

硬性約束（內核 ChapterYears::build 與 tools/validate_chapter_years.py 各查一遍）：

1. 每條判定 **evidence 非空**，且每個 eventId 都必須在 events/tongjian.json 裡真實存在；
2. yearBc 必須落在 yearRange 內，且落在事件表年份跨度內（前403–前209）。注意 BCE 數字是反的：year_bounds() 回 (earliest, latest) = (403, 209)，earliest 數值反而更大；
3. confidence 只有 high / medium，low 在反序列化階段就失敗；
4. unresolved 條目 **不得** 帶 yearBc，且 reason 必須在固定枚舉內；
5. 一章只能出現一次（decided 與 unresolved 互斥，各自也不許重名）。

## 2. 判定方法

**歸一化。** 去掉全半角標點（，。「」：；！？、『』（）《》…—・·○ 空白）之後再比對。不去標點時最長公共子串會被「。」王曰：「善。」乃」這類對話套語污染——第一版原型因此把 zhao-1-11、chu-2-08、qi-1-12 一起錯掛到前341，這是必須先去標點的直接原因。

**文本證據。** 6-gram 種子 + 雙向擴展出章文與事件正文的最長公共子串 L，記在 matched；同時算 df＝包含該歸一化片段的事件條數（識別套語：真史事只會出現一次）。按「年」聚合候選（同一年多條事件取最高分），再要求**最佳年與次佳年的分差 ≥ MARGIN**，否則是 ambiguous-multi-year。

**人物證據。** 章內 df∈[1,3] 的稀有人名（張儀 19、魏冉 14 這類高頻名不算），且這組名字只在**唯一一年**的事件裡共現 → name-window（medium）。人名優先於地名：秦、齊、楚幾乎每條事件都有，地名單獨出現不構成證據。

**注疏證據。** 從 annotations 抓 (威烈王|安王|烈王|顯王|慎靚王|愼靚王|赧王|昭襄王)+漢數字+年，±30–40 字內必須有 cue（此章/此策/當在/是歲/此年/此即/此當/附載/正當/按通鑑），並排除否定與存疑句（必非/非赧王/此當作/當作/未可/恐非）。王年→公元用 tongjian.json 自己的 (ruler, rulerYear) 映射換算。注疏同年 → 追加 commentary-date 證據並升 high；注疏與正文衝突 → commentary-conflict（判不出）。
實測：注疏裡帶王年的章共 39 章，加上 cue 限制後只剩 12 章，其中一半是「按通鑑…當作某年」的存疑句或否定句，因此該分支本輪**沒有獨立產出一章**——它只作佐證。這是誠實的結論，不是沒做。

**手動驳回（寫死在腳本裡，可審計）。** REVIEW_REJECT 六條：qi-3-01、dongzhou-18、song-wei-14、han-1-17、qi-6-08、zhongshan-10。它們都能匹配上文本，但复核不成立，見 §5。

**判不出的去向。** 一律進 unresolved 並給固定 reason；頁面顯示原因，絕不留空。

### reason 枚舉

| reason | 中文（頁面顯示） | 本輪計數 |
| --- | --- | --- |
| no-event-before-453 | 通鑑起於前403年，此前無事可繫 | 0 |
| no-tongjian-match | 通鑑沒有相近的記載 | 392 |
| below-threshold | 僅有零星字句相合，不足以為據 | 24 |
| formula-match | 只對上策文套語，非本事 | 0 |
| ambiguous-multi-year | 可繫於數年，無法取捨 | 0 |
| tongjian-bucket-year | 通鑑將此事併入前403年一條，非其真年 | 8 |
| commentary-conflict | 注疏所記之年與正文不合 | 0 |
| manual-review-reject | 复核時認定證據不成立 | 6 |

no-event-before-453 計數為 0 的原因：前453–前404 那五十年通鑑一條事件也沒有，這些章（東周策、西周策開頭若干）在文本比對裡同樣是「零相似」，與「有事但通鑑沒收」的章不可區分，程序無法誠實地單獨歸類，故統一落 no-tongjian-match。枚舉保留，留待接外部年表時再細分。

tongjian-bucket-year 的 8 章（wei-1-01/02/05/06、zhao-1-01/02/04、yan-1-13）全部因為通鑑把前455–前403 的三家滅智、魏文侯整段壓進「前403年」那 21 條事件裡。掛到 403 就是造假，所以一律拒絕。

### 閾值與覆蓋率（同一份腳本改常量重跑，實測）

| 設定 | 已判章 | coverage | 說明 |
| --- | --- | --- | --- |
| L_MIN=6 | 85 | 17.1% | 新增 18 章逐條查驗**全是套語**，否決 |
| L_MIN=7 | 70 | 14.1% | 新增 3 章仍含套語，否決 |
| **L_MIN=8（採用）** | **67** | **13.5%** | 8–9 字檔 8 章：7 對 1 錯 |
| L_MIN=9 | 63 | 12.7% | 丟掉 4 條正確判定 |
| L_MIN=10 | 61 | 12.3% | 再丟 2 條 |

L_MIN=6 被否決的樣本（matched → 章）：令人謂太子曰(chu-2-09→前314)、召大臣而謀曰(qi-1-06→前341)、再拜稽首曰臣(zhao-2-07→前307)、也王曰善乃以(zhao-3-03→前270)、靖郭君靖郭君(qi-1-05→前321)。這些是策文共用套語，不是史事。DF_MAX 在 1/3/4 之間變化對結果無影響（皆 67），故取 2。

## 3. 精度自檢：30 條抽查（seed=20261105，對最終 67 章 random.sample）

抽樣與判讀都靠人工逐條讀章文與事件正文，不是腳本自評。

| # | chapterId | 章名 | 判定年 | 置信 | 章文匹配片段 | 通鑑事件正文（節錄） | 判讀 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | han-3-15 | 趙魏攻華陽 | 273 | medium | 曰未急也穰侯怒曰 | 趙人、魏人伐韓華陽。韓人告急于秦，秦王弗救。韓相國謂陳筮曰：「事急矣，願公雖病，爲一宿之行！」陳筮如秦，見穰侯。穰侯曰：「事急乎？故使公來。」陳筮曰... | 成立 |
| 2 | qi-1-07 | 南梁之難 | 341 | medium | 結韓之親而晚承魏之 | 魏龐涓伐韓。韓請救於齊。齊威王召大臣而謀曰：「蚤救孰與晚救？」成侯曰：「不如勿救。」田忌曰：「弗救則韓且折而入於魏，不如蚤救之。」孫臏曰：「夫韓、魏... | 成立 |
| 3 | han-1-07 | 宣王謂摎留 | 321 | medium | 用犀首張儀而西河之外亡今 | 韓宣惠王欲兩用公仲、公叔爲政，問於繆留。對曰：「不可，晉用六卿而國分；齊簡公用陳成子及闞止而見殺；魏用犀首、張儀而西河之外亡。今君兩用之，其多力者內... | 成立 |
| 4 | yan-1-06 | 張儀爲秦破從連橫謂燕王 | 311 | high | 河間以事秦大王不事秦秦下甲雲中九原驅趙而攻燕則易水長... | 張儀乃北之燕，說燕王曰：「今趙王已入朝，効河間以事秦。大王不事秦，秦下甲雲中、九原，驅趙而攻燕，則易水、長城非大王之有也！且今時齊、趙之於秦，猶郡縣... | 成立 |
| 5 | qin-3-14 | 謂應侯曰君禽馬服乎 | 259 | high | 武安君爲三公君能爲之下乎雖欲無爲之下固不得 | 十月，武安君分軍爲三：王齕攻趙武安、皮牢，拔之。司馬梗北定太原，盡有上黨地。韓、魏使蘇代厚幣說應侯曰：「武安君卽圍邯鄲乎？」曰：「然。」蘇代曰：「趙... | 成立 |
| 6 | qin-3-17 | 蔡澤見逐于趙 | 255 | high | 商君吳起大夫種其爲人臣盡忠致功則可願矣閎夭 | 燕客蔡澤聞之，西入秦，先使人宣言於應侯曰：「蔡澤，天下雄辯之士；彼見王，必困君而奪君之位。」應侯怒，使人召之。蔡澤見應侯，禮又倨。應侯不快，因讓之曰... | 成立 |
| 7 | han-1-05 | 蘇秦爲楚合從說韓王 | 333 | high | 夫以大王之賢挾強韓之兵而有牛後之名臣竊爲大王羞之韓王 | 於是蘇秦說韓宣惠王曰：「韓地方九百餘里，帶甲數十萬，天下之強弓、勁弩、利劍皆從韓出。韓卒超足而射，百發不暇止。以韓卒之勇，被堅甲，蹠勁弩，帶利劍，一... | 成立 |
| 8 | qi-6-01 | 齊負郭之民有狐咺者 | 284 | high | 千乘博昌之間方數百里雨血沾衣王知之乎 | 齊王出亡之衞，衞君辟宮舍之，稱臣而共具。齊王不遜，衞人侵之。齊王去奔鄒、魯，有驕色；鄒、魯弗內，遂走莒。楚使淖齒將兵救齊，因爲齊相。淖齒欲與燕分齊地... | 成立 |
| 9 | yan-1-11 | 燕昭王收破燕後即位 | 312 | high | 亂而襲破燕孤極知燕小力少不足以報然 | 燕人共立太子平，是爲昭王。昭王於破燕之後，弔死問孤，與百姓同甘苦，卑身厚幣以招賢者。謂郭隗曰：「齊因孤之國亂而襲破燕，孤極知燕小力少，不足以報；然誠... | 成立 |
| 10 | zhao-3-13 | 秦圍趙之邯鄲 | 258 | high | 賢奪其所憎而與其所愛彼又將使其子女讒妾爲諸侯妃姬處梁... | 於是楚王使春申君將兵救趙，魏王亦使將軍晉鄙將兵十萬救趙。秦王使謂魏王曰：「吾攻趙，旦暮且下，諸侯敢救之者，吾已拔趙，必移兵先擊之！」魏王恐，遣人止晉... | 成立 |
| 11 | zhao-2-01 | 蘇秦從燕之趙始合從 | 333 | high | 竊爲大王計莫如一韓魏齊楚燕趙 | 文公從之，資蘇秦車馬，以說趙肅侯曰：「當今之時，山東之建國莫強於趙，秦之所害亦莫如趙。然而秦不敢舉兵伐趙者，畏韓、魏之議其後也。秦之攻韓、魏也，無有... | 成立 |
| 12 | qin-5-01 | 謂秦王 | 273 | medium | 三王不足四五伯不足六 | 韓、魏旣服於秦，秦王將使武安君與韓、魏伐楚，未行，而楚使者黃歇至，聞之，畏秦乘勝一舉而滅楚也，乃上書曰：「臣聞物至則反，冬、夏是也；致至則危，累棋是... | 成立 |
| 13 | wei-1-10 | 蘇子爲趙合從說魏王 | 333 | high | 萬奮擊二十萬廝徒十萬車六百乘騎五千匹 | 蘇秦說魏王曰：「大王之地方千里，地名雖小，然而田舍廬廡之數，曾無所芻牧。人民之衆，車馬之多，日夜行不絕，輷輷殷殷，若有三軍之衆。臣竊量大王之國不下楚... | 成立 |
| 14 | yan-1-10 | 初蘇秦弟厲因燕質子而求見齊王 | 316 | medium | 燕相子之與蘇代婚 | 蘇秦旣死，秦弟代、厲亦以遊說顯於諸侯。燕相子之與蘇代婚，欲得燕權。蘇代使於齊而還，燕王噲問曰：「齊王其霸乎？」對曰：「不能。」王曰：「何故？」對曰：... | 成立 |
| 15 | wei-1-11 | 張儀爲秦連橫說魏王 | 317 | high | 成亦明矣大王不事秦秦下兵攻河外 | 張儀說魏襄王曰：「梁地方不至千里，卒不過三十萬，地四平，無名山大川之限，卒戍楚、韓、齊、趙之境，守亭、障者不過十萬，梁之地勢固戰場也。夫諸侯之約從，... | 成立 |
| 16 | qi-6-03 | 燕攻齊取七十餘城 | 250 | medium | 書約之矢以射城中遺燕將 | 燕將攻齊聊城，拔之。或譖之燕王，燕將保聊城，不敢歸。齊田單攻之，歲餘不下。魯仲連乃爲書，約之矢以射城中，遺燕將，爲陳利害曰：「爲公計者，不歸燕則歸齊... | 成立 |
| 17 | qin-3-09 | 范雎至秦 | 270 | high | 王而所願陳者皆匡君之事處人骨肉之間願 | 秦謁者王稽使於魏，范睢夜見王稽。稽潛載與俱歸，薦之於王，王見之於離宮。范睢佯爲不知永巷而入其中，王來而宦者怒逐之，曰：「王至！」范睢謬曰：「秦安得王... | 成立 |
| 18 | qi-1-08 | 成侯鄒忌爲齊相 | 341 | medium | 市曰我田忌之人也 | 成侯鄒忌惡田忌，使人操十金，卜於市，曰：「我，田忌之人也。我爲將三戰三勝，欲行大事，可乎？」卜者出，因使人執之。田忌不能自明，率其徒攻臨淄，求成侯；... | 成立 |
| 19 | han-1-06 | 張儀爲秦連橫說韓王 | 311 | high | 夫戰孟賁烏獲之士以攻不服之弱國無 | 張儀遂之韓，說韓王曰：「韓地險惡山居，五穀所生，非菽而麥，國無二歲之食；見卒不過二十萬。秦被甲百餘萬。山東之士被甲蒙胄以會戰，秦人捐甲徒裼以趨敵，左... | 成立 |
| 20 | yan-2-09 | 昌國君樂毅爲燕昭王合五國之兵而攻齊 | 279 | high | 歸趙將軍自爲計則可矣而亦何以報先王之所以遇將軍之意乎 | 趙王封樂毅於觀津，尊寵之，以警動於燕、齊。燕惠王乃使人讓樂毅，且謝之曰：「將軍過聽，以與寡人有隙，遂捐燕歸趙。將軍自爲計則可矣，而亦何以報先王之所以... | 成立 |
| 21 | chu-1-04 | 昭奚恤與彭城君議于王前 | 353 | medium | 昭奚恤、江乙 | 楚昭奚恤爲相。江乙言於楚王曰：「人有愛其狗者，狗嘗溺井，其鄰人見，欲入言之，狗當門而噬之。今昭奚恤常惡臣之見，亦猶是也。且人有好揚人之善者，王曰：『... | 成立 |
| 22 | yan-3-03 | 燕王喜使栗腹以百金爲趙孝成王壽 | 251 | medium | 長平其孤未壯可伐也王 | 燕王喜使栗腹約歡於趙，以五百金爲趙王酒。反而言於燕王曰：「趙壯者皆死長平，其孤未壯，可伐也。」王召昌國君樂閒問之，對曰：「趙四戰之國，其民習兵，不可... | 成立 |
| 23 | qi-1-03 | 靖郭君將城薛 | 321 | high | 大魚乎網不能止鉤不能牽蕩而失水則螻蟻 | 靖郭君欲城薛，客謂靖郭君曰：「君不聞海大魚乎？網不能止，鉤不能牽，蕩而失水，則螻蟻制焉。今夫齊，亦君之水也。君長有齊，奚以薛爲！苟爲失齊，雖隆薛之城... | 成立 |
| 24 | qi-1-16 | 蘇秦爲趙合從說齊宣王 | 333 | high | 十日而戰勝存亡之機決矣韓魏戰而勝秦則兵半折四境不守戰... | 蘇秦說齊王曰：「齊四塞之國，地方二千餘里，帶甲數十萬，粟如丘山。三軍之良，五家之兵，進如鋒矢，戰如雷霆，解如風雨，卽有軍役，未嘗倍泰山、絕清河、涉渤... | 成立 |
| 25 | chu-1-08 | 江乙惡昭奚恤 | 353 | medium | 狗嘗溺井其鄰人見 | 楚昭奚恤爲相。江乙言於楚王曰：「人有愛其狗者，狗嘗溺井，其鄰人見，欲入言之，狗當門而噬之。今昭奚恤常惡臣之見，亦猶是也。且人有好揚人之善者，王曰：『... | 成立 |
| 26 | qi-6-05 | 貂勃常惡田單 | 279 | high | 王上者孰與周文王王曰吾不若也貂勃曰然臣固知王不若也下... | 田單任貂勃於王。王有所幸臣九人，欲傷安平君，相與語於王曰：「燕之伐齊之時，楚王使將軍將萬人而佐齊。今國已定而社稷已安矣，何不使使者謝於楚王？」王曰：... | 成立 |
| 27 | chu-1-09 | 江乙欲惡昭奚恤于楚 | 353 | high | 以王好聞人之美而惡聞人之惡也王曰善寡人願兩聞之 | 楚昭奚恤爲相。江乙言於楚王曰：「人有愛其狗者，狗嘗溺井，其鄰人見，欲入言之，狗當門而噬之。今昭奚恤常惡臣之見，亦猶是也。且人有好揚人之善者，王曰：『... | 成立 |
| 28 | qi-6-04 | 燕攻齊齊破 | 279 | high | 對曰王不如因以爲己善王嘉單之善下令曰寡人憂民之飢也單... | 田單相齊，過淄水，有老人涉淄而寒，出水不能行。田單解其裘而衣之。襄王惡之，曰：「田單之施於人，將以取我國乎！不早圖，恐後之變也。」左右顧無人，巖下有... | 成立 |
| 29 | qin-2-01 | 齊助楚攻秦 | 313 | medium | 商於之地不可得而 | 秦王欲伐齊，患齊、楚之從親，乃使張儀至楚，說楚王曰：「大王誠能聽臣，閉關絕約於齊，臣請獻商於之地六百里，使秦女得爲大王箕帚之妾，秦、楚嫁女娶婦，長爲... | 成立 |
| 30 | zhao-3-10 | 秦攻趙于長平 | 259 | high | 不遺餘力矣必以倦而歸也虞卿曰秦以其力攻其所不能取倦而... | 趙王將使趙郝約事於秦，割六縣。虞卿謂趙王曰：「秦之攻王也，倦而歸乎？王以其力尙能進，愛王而弗攻乎？」王曰：「秦不遺餘力矣，必以倦而歸也。」虞卿曰：「... | 成立 |

**結論：30 條中成立 30 條，精度 100%（≥ 90% 硬線）。** 樣本 high 20 / medium 10，與全庫 41/26 的比例相符。
另對最弱的 8–9 字檔單獨复核了全部 8 章：7 對 1 錯（錯的 han-1-17 已驳回），說明 L_MIN=8 正好卡在噪聲邊界上。

## 4. 三條最硬的判定（可直接拿原文與通鑑核對）

1. **qi-6-05 貂勃常惡田單 → 前279**，L=56，matched＝「王上者孰與周文王王曰吾不若也貂勃曰然臣固知王不若也下」；事件 tj-y0279-p86「田單任貂勃於王。王有所幸臣九人，欲傷安平君…」。
2. **qi-6-04 燕攻齊齊破 → 前279**，L=53，matched＝「對曰王不如因以爲己善王嘉單之善下令曰寡人憂民」；事件「田單相齊，過淄水，有老人涉淄而寒…」。
3. **zhao-3-13 秦圍趙之邯鄲 → 前258**，L=47，matched＝「賢奪其所憎而與其所愛彼又將使其子女讒妾爲諸侯妃姬」；事件「於是楚王使春申君將兵救趙，魏王亦使將軍晉鄙將兵十萬救趙…」。

同級可核：qi-6-09 齊王建入朝于秦 前222（L38）、zhao-4-18 趙太后新用事 前265（L36）、zhao-3-10 秦攻趙于長平 前259（L36）、qin-1-07 司馬錯與張儀爭論 前316（L34）。

## 5. 曾被攔下的判錯（复核不是走過場）

1. **han-1-17 秦韓戰于濁澤 曾判前313**：L=8，命中的片段是「賂之以一名都與之」——那是張儀誑楚（前313），而濁澤之戰在前307。僅靠分數會放它過關，是人工复核才看出語境錯位。→ manual-review-reject。
2. **qi-6-08 齊閔王之遇殺 曾判前283**：章本事是閔王前284 之死，匹配到的却是法章即位的次年（前283）敘述，差一年。對「某王遇殺」這種章，差一年就是掛錯。→ reject。
3. **zhongshan-10 昭王既息民繕兵 曾判前258**：唯一證據是套語「楚地方五千里持戟百萬」，靠 df=1 的運氣過線，實際上它與前258 的平原君／毛遂無關。→ reject。
4. 更早一輪還駁回 3 條：qi-3-01 楚王死（章講懷王死於前296，前299 只是他被扣）、dongzhou-18 蘇厲爲周最謂蘇秦（把蘇秦放在前310，而通鑑記其死於前316）、song-wei-14 胥靡（本事屬嗣君初立，前283 只是通鑑收其軼事之年）。
5. 另有一條**閾值設計錯誤**被實測擋下：早期把「人物生卒連續性」當硬約束套在文本匹配上，殺掉了 zhao-3-13/前258、qin-1-05/前233、qi-6-03/前250 等 12 條正確判定。教訓寫進腳本註釋：通鑑只記有事之年，人物在某年出現不代表他當年才在世，不能用出現年份當生卒區間。該約束現在只用於 name-window 分支。

## 6. 內核與頁面

- crates/zgc-core/src/chapter_years.rs：ChapterYears::load(book_dir, &Events)（文件缺失 → 空表優雅降級；文件壞 → 硬錯誤）、build 做 §1 全部 fail-fast 校驗，錯誤枚舉帶中文 Display；reason_text(reason) 供頁面用。
- crates/zgc-app：App 新增 chapter_years 欄位；with_events 簽名不變（既有測試不動），新增鏈式 with_chapter_years；main.rs 啟動時載入並打印「系年：已判 N 章 · 未判 M 章」。
- 章頁（/read?p=<章首段>，position.paragraph == 0 時）插入 section.chap-when：
  - 有年份：「約前279年 · 置信度 高」+ details/summary「憑什麼是這一年」，逐條列出證據 kind、章文片段、通鑑事件正文節錄，並附 <a href="/timeline?e=tj-y0279-p86">通鑑前279年</a> 可跳到時間軸定位；
  - 判不出：「未能系年」+ 原因（如「通鑑沒有相近的記載」）；
  - 沒有系年文件時整塊不渲染。
  - 全程零 JavaScript，只用 details。

## 7. 開發中發現並修掉的 bug（值得記一筆）

Events::year_bounds() 回的是 (earliest, latest) = (403, 209)——BCE 數字反向，earliest 反而更大。第一版校驗寫成 year_bc < earliest || year_bc > latest，於是**所有**年份都被判成越界：單元測試用的是單一年份 (333,333) 的 fixture，> 與 < 恰好等價，測試全綠；直到真的起服務載入真數據才報「章「chu-1-03」的年份前353年不在事件表範圍前403–前209年內」。
修法：改成 year_bc > earliest || year_bc < latest，並補兩條回歸測試（跨度 fixture：前353 接受、前404 拒絕）。教訓：邊界校驗的 fixture 一定要用**真實跨度**，等寬區間會讓反向比較隱身。

## 8. 驗收證據

cargo test --workspace（原始輸出，尾行）：

~~~
     Running unittests src/main.rs (target/debug/deps/zgc_app-…)
test result: ok. 70 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
     Running unittests src/lib.rs (target/debug/deps/zgc_core-…)
test result: ok. 70 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
     Running tests/data_integration.rs
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
     Running tests/entities_data.rs
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
     Running tests/load_fixtures.rs
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
   Doc-tests zgc_core
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
~~~

合計 165 passed / 0 failed（原 152 全綠 + 13 條新測試）。cargo build --release：Finished release profile [optimized]，0 warning。

tools/validate_chapter_years.py：

~~~
chapters=497 decided=67 unresolved=430 coverage=13.5%
OK: every decided year has real, non-empty evidence and sits inside the timeline
~~~

curl 有年份的章（/read?p=qi-6-05-p0001）：

~~~html
<section class="chap-when"><p class="when">約前279年<span class="conf">置信度 高</span></p><details><summary>憑什麼是這一年</summary><ul class="why"><li><p class="kind">章文與通鑑記事逐字相合</p><p class="match">章文「王上者孰與周文王王曰吾不若也貂勃曰然臣固知王不若也下者孰與齊桓公王曰吾不若也貂勃…」</p><p class="ev">通鑑：田單任貂勃於王。王有所幸臣九人，欲傷安平君，相與語於王曰：「燕之伐齊之時，楚王使將軍將萬人而佐齊。今國已定而社稷已安矣，… <a href="/timeline?e=tj-y0279-p86">通鑑前279年</a></p></li><li><p class="kind">同一組人物共見於此年</p>…</li></ul></details></section>
~~~

curl 判不出的章（/read?p=dongzhou-01-p0001）：

~~~html
<section class="chap-when"><p class="when unknown">未能系年<span class="why-hint">通鑑沒有相近的記載</span></p></section>
~~~

/timeline?e=tj-y0279-p86 回 200，事件上帶 class="tl-ev focus"，即證據連結可直達。

## 9. 文件清單

新增：
- tools/build_chapter_years.py（系年構建，含 REVIEW_REJECT 與閾值常量）
- tools/validate_chapter_years.py（獨立校驗器）
- data/zhanguoce/events/chapter_years.json（67 decided / 430 unresolved）
- crates/zgc-core/src/chapter_years.rs（模塊 + 13 條內聯測試）
- docs/M3C2_NOTES.md（本文件）

修改：
- crates/zgc-core/src/lib.rs（mod + pub use）
- crates/zgc-app/src/app.rs（chapter_years 欄位、with_chapter_years、getter）
- crates/zgc-app/src/main.rs（載入、啟動統計、耗時行）
- crates/zgc-app/src/pages.rs（chapter_when / evidence_kind_label / clip_escape）
- crates/zgc-app/src/style.css（.chap-when 樣式）

未改動：data/zhanguoce/events/tongjian.json、book.json、volumes/**、entities/**、crates/zgc-core/tests/**、real_data_tests.rs 既有測試，以及 docs 下其餘文件。

## 10. 已知限制 / 後續

- 覆蓋率 13.5% 是源的性質所致：通鑑只記有事之年，且前453–前404 完全空白；中山策、宋衛策、部分楚策幾乎不被提及。要提覆蓋率必須引入通鑑以外的年表（如《史記》六國年表），那是另一個數據任務，不能用放寬閾值來冒充。
- name-window 分支（6 章）只保證「這組人只在這一年共見」，年份對，但章內敘事的具體月份無法細化。
- 少數高 L 判定（qin-1-05 張儀說秦王→前233）反映的是通鑑把同文本記在該年的事實，策文本身的作者／年代歸屬仍是學術懸案，已在證據裡如實給出 matched 與事件，讀者可自行判斷。

## 11. M3c-2b：把姓名拼接從「引文」的位置上拿下來

### 11.1 缺陷

name-overlap / name-window 這 59 條證據，`matched` 存的是**逐個姓名直接拼接**的字串（chu-1-08 `"昭奚恤江乙"`、qin-1-07 `"司馬錯張儀秦惠王"`、zhao-3-13 `"平原君魯仲連"`），頁面卻按「章文『…』」渲染它。章文裡根本不存在這個字串——這等於用假引文包裝真結論：讀者以為自己可以回原文核對，實際核對不到。結論本身沒錯，錯的是讓它看起來像原文。

### 11.2 改法

- 資料層（tools/build_chapter_years.py）：name-* 證據改輸出 `"names": ["昭奚恤", "江乙"]`，**不再寫 `matched`**；substring / substring-range / commentary-date 仍寫 `matched`。姓名按 `sorted()` 排序後輸出，消除此前由 set 走訪順序造成的重建不穩定（重建 diff 從 24 行降到 1 行，且僅是姓名順序）。
- 校驗器（tools/validate_chapter_years.py）：新增形狀斷言——name-* 必須有非空 `names` 且不得有 `matched`；**每個姓名都必須真的出現在該章正文裡**（去標點後子串匹配）；文字類證據必須有 `matched` 且不得有 `names`。校驗器接受一個可選路徑參數，負面測試就是靠它跑的。
- 內核（crates/zgc-core/src/chapter_years.rs）：`Evidence.text: EvidenceText`，枚舉 `Matched(String) | Names(Vec<String>)`，用自訂 `Deserialize` 保證**恰好一種形狀**：兩者皆無、兩者皆有、空白 `matched`、空 `names` 全部在解析期報錯。kind 與形狀的耦合留在 Python 校驗器，內核不重複一份 kind 詞表。
- 頁面（crates/zgc-app/src/pages.rs）：`Names` → `<p class="match names">共現人物：昭奚恤、江乙</p>`（無引號、不稱章文）；`Matched` 且 kind 為 substring* → 「章文『…』」照舊；`Matched` 的其他 kind（commentary-date）→ 改標「注疏『…』」，因為它引的是注疏而非章文，屬於同一類假框架（目前 0 條）。

### 11.3 數字不變（只換表達方式）

| | 重建前 | 重建後 |
|---|---|---|
| decided / unresolved | 67 / 430 | 67 / 430 |
| coverage | 13.5% | 13.5% |
| high / medium | 41 / 26 | 41 / 26 |
| 證據條數 | 121（substring 61, substring-range 1, name-overlap 53, name-window 6） | 121（同上） |

逐條比對：chapterId、yearBc、yearRange、confidence、method、unresolved reason 全部一字不變；59 條 name-* 的 `matched` 全部換成 `names`，1 條姓名順序因排序而變；61 條 substring 與 1 條 substring-range 的 `matched` 一字不變。

### 11.4 校驗器負面測試

把 chu-1-08 的姓名表塞進一個原章沒有的名字「公孫杵臼」：

```
$ python3 tools/validate_chapter_years.py /tmp/bad_name.json
chapters=497 decided=67 unresolved=430 coverage=13.5%
FAIL: 1 problem(s)
  - chu-1-08: name-overlap evidence tj-y0353-p46 names '公孫杵臼', which is not in this chapter
exit=1
```

另外兩種形狀錯也各自 exit=1：name-* 帶 `matched`（"carries matched; joined names are not a quotation"）、substring 帶 `names`（"carries names; text evidence must quote matched"）。

### 11.5 頁面實測

name-* 章（/read?p=chu-1-08-p0001，注意第二條）：

```html
<li><p class="kind">章文與通鑑記事逐字相合</p><p class="match">章文「狗嘗溺井其鄰人見」</p><p class="ev">通鑑：楚昭奚恤爲相。…</p></li>
<li><p class="kind">同一組人物共見於此年</p><p class="match names">共現人物：昭奚恤、江乙</p><p class="ev">通鑑：楚昭奚恤爲相。…</p></li>
```

substring 章（/read?p=qi-6-05-p0001）第一條仍是引號包裹的章文片段：

```html
<li><p class="kind">章文與通鑑記事逐字相合</p><p class="match">章文「王上者孰與周文王王曰吾不若也貂勃曰然臣固知王不若也下者孰與齊桓公王曰吾不若也貂勃…」</p>
```

### 11.6 本輪改動文件

- tools/build_chapter_years.py（name-* 輸出 names[]，姓名排序）
- tools/validate_chapter_years.py（形狀斷言 + 姓名在章斷言 + 可選路徑參數）
- data/zhanguoce/events/chapter_years.json（重建）
- crates/zgc-core/src/chapter_years.rs（EvidenceText + 自訂 Deserialize + 4 條新測試）
- crates/zgc-core/src/lib.rs（pub use EvidenceText）
- crates/zgc-app/src/pages.rs（chapter_when 按形狀分流）
- docs/M3C2_NOTES.md（本節）

未動：tongjian.json、book.json、volumes/**、entities/**、其他 core 模塊、app.rs / main.rs、style.css、crates/zgc-core/tests/**。

