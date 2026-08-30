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
   │   手机/电脑浏览器 (PWA)           ← 复习 / 查词 / 添加生词
   │
   └─ lexicon-cli mcp <db>            ← MCP（Model Context Protocol）服务器
          ▼
       AI 助手（Claude Desktop / Cursor / Claude Code 等）以工具调用方式
       直接查词、取复习队列、评分、管词表与学习记录
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

### 4. 接入 AI 助手（MCP）

`lexicon-cli mcp <db>` 把词典与复习功能暴露为 MCP 工具（JSON-RPC 2.0 over stdio），
Claude Desktop、Cursor、Claude Code 等 MCP 客户端可以直接调用：

    lexicon-cli mcp oxford.db

**Claude Desktop 配置示例**（`claude_desktop_config.json`）：

    {
      "mcpServers": {
        "lexicon": {
          "command": "/home/az/lexicon/target/release/lexicon-cli",
          "args": ["mcp", "/home/az/lexicon/oxford.db"]
        }
      }
    }

**Cursor**：Settings → MCP → Add new MCP server → Command 填
`/home/az/lexicon/target/release/lexicon-cli mcp /home/az/lexicon/oxford.db`。

暴露的 15 个工具：

| 工具 | 说明 |
|------|------|
| `lookup` | 查词：音标、词性、中文释义、例句、固定搭配 |
| `search` | 前缀搜索词条 |
| `stats` | 词典规模 + 今日到期数 |
| `add_card` | 把单词/词组/句式加入复习（可设难度与类型） |
| `due_queue` | 今日复习队列（受每日新学/复习上限约束，可限定词表） |
| `grade_card` | 为卡片评分并更新间隔重复排程（0-3） |
| `card_history` | 单卡复习历史 |
| `card_records` | 学习记录列表（状态/次数/遗忘/到期/难度） |
| `set_difficulty` | 标记卡片难度（影响日后题面） |
| `wordlists` | 列出词表（高考/雅思等） |
| `settings_get` / `settings_set` | 每日新学数、复习上限、当前词表 |
| `random_word` | 随机抽词加入复习 |
| `phrase_count` / `phrases_for` | 词组/句式库规模与查询 |

三种考察模式（题面由 `due_queue` 返回）：

- **单词·简单**：中文释义/义项 + 例句中文翻译作提示 → 回忆单词
- **单词·困难**：只给首字母 + 中文义项 → 回忆单词
- **词组/句式**：中文释义 + 中文例句 → 回忆整个固定搭配或句式

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
| GET  | /api/due?limit=N | 到期卡片（定义为清洗后的纯文本） |
| POST | /api/grade | {id, grade: 0-3} 评分 |
| GET  | /api/search?q= | 前缀搜索词条 |
| GET  | /api/lookup?w= | 查词（解析 @@@LINK= 重定向） |
| POST | /api/add | {word} 加入复习 |
| GET  | /manifest.json, /sw.js, /icon.svg | PWA 资源 |

## 核心 crate

- `crates/core`（lexicon-core）：独立 MDX 解析器、SM2 调度、SQLite 存储（捆绑编译，无系统依赖）
- `crates/cli`（lexicon-cli）：CLI + 内置 HTTP/PWA 服务器 + MCP 服务器（`mcp.rs`，stdio JSON-RPC，零额外依赖）

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
