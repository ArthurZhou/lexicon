# Lexicon

本地词典 + 间隔重复（SM2）背单词引擎。纯本地运行，无云端，数据全部在设备上的 SQLite 中。

## 架构

~~~
oxford.mdx (MDict 词典, 20MB)
   │  lexicon-cli import              ← 解析 MDX（MDict 格式，zlib/无压缩）
   ▼
oxford.db (SQLite)                   ← 自有格式：词条 + 间隔重复卡片
   │
   ├─ lexicon-cli serve               ← 内置 HTTP/PWA
   │      ▼
   │   手机/电脑浏览器 (PWA)           ← 复习 / 查词 / 添加生词 / 学词组
   │
   └─ lexicon-cli review / wordlist   ← 命令行复习与批量导入词表
~~~

**是否需要把 MDX 转为自己的格式？—— 需要，`import` 已经做完了这件事。**

- MDX 是词典的原始打包格式（HTML 定义 + 图片/音频资源），解析慢、体积大（你的 oxford.mdx 20MB）。
- 转为 SQLite（约 180MB，含完整定义）后，查询是毫秒级索引查询，且**运行时不再依赖任何 MDX 解析逻辑**。
- 对移动端 App：SQLite 可以直接打进 App（或放在设备文件系统里），无需在手机上跑解析器。
- 转换是一次性的：`lexicon-cli import oxford.mdx oxford.db`，之后 app 只需读 oxford.db。

## 构建

~~~
cargo build --release
# 产物在 target/release/lexicon-cli
~~~

## 使用

### 1. 导入词典（转自有格式）

~~~
lexicon-cli import oxford.mdx oxford.db
# imported 119563 entries into oxford.db
~~~

### 2. 在手机上用（WebUI / PWA）

~~~
lexicon-cli serve oxford.db --port 8000
~~~

启动后会打印局域网地址（服务监听 `0.0.0.0`）：

~~~
lexicon UI listening on http://0.0.0.0:8000
  mobile (same Wi-Fi): http://192.168.0.90:8000
  this computer:       http://127.0.0.1:8000
~~~

手机与电脑连同一 Wi-Fi，用手机浏览器打开上面的地址：

- **复习**：显示到期卡片 → 点击“显示释义” → 选 再次/困难/认识/简单（SM2 调度）
- **查词**：搜索前缀 → 点词条看释义 → “加入复习”
- 没有到期卡片时可以“抽一张生词看看”

### 3. 作为手机 App 安装（PWA）

PWA（manifest + service worker + 图标）已内置。把页面“添加到主屏幕”即可像 App 一样全屏打开：

- **Android (Chrome)**：菜单 → “安装应用” / 添加到主屏幕
- **iOS (Safari)**：分享 → “添加到主屏幕”
- 局域网 HTTP 下 service worker 受浏览器安全策略限制（需要 HTTPS 或 localhost 才注册离线缓存）；若想离线可用，把服务暴露为 HTTPS（如 Tailscale 隧道 / cloudflare tunnel / Caddy 反代）。

### 4. 学习与复习（中译英，难度自适应）

复习题面按卡片「难度档位」自动匹配你的熟悉程度（SM2 评分会自动升降档，
也可以在卡片上手动切换）：

| 档位 | 单词题面 | 词组/句式题面 |
|------|----------|----------------|
| 简单 | 挖空例句 + 该词中文意思 + 例句中文翻译 | 中文释义 + 挖空例句 + 例句翻译 |
| 中等 | 挖空例句 + 例句中文翻译（不给词义） | 中文释义 + 例句中文翻译（不给英文例句） |
| 困难 | 首字母 + 字母数 + 中文意思，不给例句 | 词组首字母骨架 + 中文释义 |

- **背单词**：看中文回忆英文，`忘记得多` 自动降档给更多提示，`连续答简单` 自动升档。
- **固定搭配与词组**：`phrases --extract` 从词典提取词组/习语库；查词页可直接
  把某个词的词组/习语一键加入复习，词组卡与单词卡一起进入 SM2 队列。

### 5. 命令行

~~~
lexicon-cli lookup oxford.db apple        # 查词（自动解析 @@@LINK= 别名）
lexicon-cli review oxford.db apple ...    # 交互式复习指定词
lexicon-cli scope oxford.db wordlist.txt  # 从文件批量加入生词
lexicon-cli wordlist oxford.db 高考 list.txt --type word   # 建词表（word/phrase/pattern）
lexicon-cli phrases oxford.db break       # 看某词的词组/习语
lexicon-cli phrases oxford.db --extract   # 全量提取词组/句式库（约 1.6 万条）
lexicon-cli config oxford.db --new 20 --review 100   # 每日新学/复习上限
lexicon-cli stats oxford.db               # 统计
~~~

## API

| 方法 | 路径 | 说明 |
|------|------|------|
| GET  | / | 移动端 PWA 页面 (text/html) |
| GET  | /api/stats | {words, cards, due} |
| GET  | /api/due?limit=N&list=W | 当日到期队列（题面按难度档位生成） |
| POST | /api/grade | {id, grade: 0-3} 评分（自动升降档） |
| POST | /api/card/difficulty | {id, difficulty: easy/medium/hard} 手动调档 |
| GET  | /api/card/history?id= | 卡片复习历史 |
| GET  | /api/search?q= | 前缀搜索词条 |
| GET  | /api/lookup?w= | 查词（解析 @@@LINK= 重定向） |
| POST | /api/add | {word} 加入复习 |
| POST | /api/random | 随机抽词加入复习 |
| GET  | /api/phrases?limit=&offset= | 浏览词组库 |
| GET  | /api/phrases/for?word= | 某词的词组/习语/句式列表 |
| POST | /api/phrase/add | {source, text, type} 加入词组/句式卡 |
| POST | /api/phrases/rebuild | 从词典全量重建词组库 |
| GET/POST | /api/settings | 每日新学/复习上限、当前词表 |
| GET  | /api/wordlists | 词表列表 |
| GET  | /api/cards?limit= | 学习记录 |
| GET  | /manifest.json, /sw.js, /icon.svg | PWA 资源 |

## 核心 crate

- `crates/core`（lexicon-core）：MDX 解析器、OALECD9 释义结构化提取、题面生成（挖空/首字母）、SM2 调度与难度自适应、SQLite 存储（捆绑编译，无系统依赖）。全部业务逻辑都在这里。
- `crates/cli`（lexicon-cli）：只有交互层——CLI 参数解析、终端复习、HTTP 路由；页面 UI 在 `crates/cli/assets/`（HTML/CSS/JS 与 Rust 分离，`include_str!` 打包）。

## 数据模型

- `entries`: headword + 原始 HTML 定义
- `words`: 词表（用于前缀搜索）
- `cards`: SM2 卡片（due/interval/ease/reps/lapses/status + 类型/难度/词组文本/来源）
- `review_log`: 每次复习评分记录（含是否首学）
- `wordlists` / `wordlist_words`: 词表与成员（限定学习范围）
- `settings`: 每日新学数 / 复习上限 / 当前词表
- `phrases`: 从词典提取的词组/习语/句式库（含中文释义与例句）

## 已知说明

- 定义清洗（`plainify`）把词典 HTML 转为易读纯文本；音标/词性/释义/例句/中文翻译按行排布，图片与 `sound://` 音频链接会被移除（未随词典提供资源文件）。
- `@@@LINK=` 重定向（如 `10p` → ten pence）在查词时自动解析。
