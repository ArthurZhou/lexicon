# Lexicon

本地词典 + 间隔重复（SM2）背单词引擎。纯本地运行，无云端，数据全部在设备上的 SQLite 中。

## 架构

~~~
oxford.mdx (MDict 词典, 20MB)
   │  lexicon-cli import              ← 解析 MDX（MDict 格式，zlib/无压缩）
   ▼
oxford.db (SQLite)                   ← 自有格式：词条 + 间隔重复卡片
   │  lexicon-cli serve               ← 内置 HTTP/PWA
   ▼
手机/电脑浏览器 (PWA)                ← 复习 / 查词 / 添加生词
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

### 4. 命令行

~~~
lexicon-cli lookup oxford.db apple        # 查词（自动解析 @@@LINK= 别名）
lexicon-cli review oxford.db apple ...    # 交互式复习指定词
lexicon-cli scope oxford.db wordlist.txt  # 从文件批量加入生词
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
- `crates/cli`（lexicon-cli）：CLI + 内置 HTTP/PWA 服务器

## 数据模型

- `entries`: headword + 原始 HTML 定义
- `words`: 词表（用于前缀搜索）
- `cards`: SM2 卡片（due/interval/ease/reps/lapses/status）

## 已知说明

- 定义清洗（`plainify`）把词典 HTML 转为易读纯文本；音标/词性/释义/例句/中文翻译按行排布，图片与 `sound://` 音频链接会被移除（未随词典提供资源文件）。
- `@@@LINK=` 重定向（如 `10p` → ten pence）在查词时自动解析。
