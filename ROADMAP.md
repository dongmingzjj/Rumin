# MiniBrowser 项目路线图

> 生成日期: 2025-05-31
> 状态: Phase 1 MVP 完成，规划 Phase 2

## 一、Phase 1 成果盘点

### 已完成能力 ✅
| 能力 | 状态 | 备注 |
|------|------|------|
| HTTP/HTTPS 请求 | ✅ | reqwest + rustls-tls |
| HTML 解析 | ✅ | html5ever |
| DOM 树构建 | ✅ | slotmap + 自定义 Node |
| CSS 选择器 | ✅ | 支持 tag/class/id/attr/child/descendant/:first-child/:last-child |
| JS 执行 | ⚠️ | boa_engine 0.19 — 能跑简单 JS，不能处理现代语法 |
| Cookie 管理 | ✅ | 基础 jar，Set-Cookie 解析 |
| 指纹配置 | ⚠️ | 仅 UA 字符串，无真实 TLS 指纹 |
| CLI 工具 | ✅ | navigate + eval + selector + title + source + dom |
| 二进制体积 | ✅ | release 7.4MB (LTO + strip) |

### 实测发现的问题
1. **boa_engine 对现代 JS 语法支持差** — HN 页面 JS 报错 `expected token '(', got '-' in method definition`，说明不支持 ES2015+ 对象方法简写、可选链等
2. **无异步 JS 执行** — 没有 setTimeout/fetch/XHR 桥接，SPA 完全无法工作
3. **UA 泄露身份** — `MiniBrowser/0.1` 一眼假，且 HTTP 头指纹暴露 rustls
4. **无重定向跟随** — 默认跟随，但无 3xx 处理日志

---

## 二、竞品对标分析

### 2.1 vs Playwright / Puppeteer

| 维度 | Playwright | MiniBrowser (现状) | 差距 |
|------|-----------|-------------------|------|
| 浏览器引擎 | Chromium/Firefox/WebKit | boa_engine (JS) | **巨大** — 无 CSS 渲染、无 DOM 补丁 |
| JS 兼容性 | V8/SpiderMonkey 完整 | boa_engine 仅 ES5+ | 差距极大 |
| 反检测 | stealth 插件、CDP 注入 | 仅 UA | 无法对抗任何检测 |
| 性能 | 启动 ~2s, 内存 ~100MB | 启动 <100ms, 内存 <10MB | **我们赢** |
| 二进制体积 | ~300MB (Chromium) | 7.4MB | **我们赢** |
| 部署复杂度 | 需安装浏览器 | 单二进制 | **我们赢** |
| 适用场景 | 完整浏览器自动化 | 轻量爬虫 | 不同赛道 |

**结论**: 不和 Playwright 正面竞争。Playwright 是 "完整浏览器"，我们是 "够用的爬虫引擎"。

### 2.2 vs curl_cffi / httpx + parsel

| 维度 | curl_cffi | MiniBrowser (现状) | 我们多什么 |
|------|-----------|-------------------|-----------|
| TLS 指纹 | ✅ Chrome 模拟 | ❌ rustls 原始指纹 | curl_cffi 赢 |
| HTTP 层反检测 | ✅ JA3/JA4 模拟 | ❌ | curl_cffi 赢 |
| HTML 解析 | ❌ 需外挂 | ✅ 内置 html5ever | **我们赢** |
| DOM 操作 | ❌ 无 | ✅ CSS 选择器 | **我们赢** |
| JS 执行 | ❌ 无 | ⚠️ boa_engine | 部分优势 |
| Cookie 管理 | ✅ | ✅ | 平手 |
| 单二进制 | ❌ Python 依赖 | ✅ | **我们赢** |
| 部署 | pip + 系统依赖 | 单文件 | **我们赢** |

**结论**: 我们比 curl_cffi 多了 DOM/JS/选择器的内置能力，但 TLS 指纹是短板。

### 2.3 独特定位

```
Playwright ─────────────────── 全功能浏览器自动化 (重)
  ↕ 差距巨大
MiniBrowser ────────────────── 轻量"准浏览器"爬虫引擎 (中)  ← 我们的赛道
  ↕ 我们比它多了 DOM/JS
curl_cffi / httpx ──────────── HTTP 客户端 + TLS 指纹 (轻)
```

**一句话定位**: "不需要真实浏览器就能完成 80% 爬虫任务的单二进制 Rust 引擎"

---

## 三、Phase 2 路线图 (按优先级排序)

### 🏆 Tier 1: 必须做 — 爬虫核心刚需 (投入产出比最高)

#### P2-1: TLS 指纹模拟 (2-3 周)
**为什么第一**: 这是区分 "能用" 和 "能打" 的分水岭。curl_cffi 的成功全靠这个。
- 替换 reqwest 的 rustls 为 **boring** (BoringSSL Rust 绑定)
- 实现 Chrome/Firefox/Safari 的 TLS ClientHello 指纹模拟
- 支持 cipher suites、extensions、ALPN、压缩方法顺序
- 配置文件驱动，预置 Chrome 120/125/130 等主流版本指纹

**验证站**: `tls.browserleaks.com`, `www.httpbin.org/headers`

#### P2-2: HTTP 头指纹对齐 (1 周)
- 自动匹配 UA 对应的 Accept / Accept-Language / Accept-Encoding / Sec-Ch-Ua 等
- Chrome 的 `Sec-CH-UA` Client Hints 头
- 缺失头会被 Cloudflare 等 WAF 直接拒绝

**验证站**: `httpbin.org/headers`, `bot.sannysoft.com`

#### P2-3: fetch / XHR 桥接 (1-2 周)
- 在 boa_engine 中注入 `fetch()` 函数，桥接到 mb-network
- 支持 Promise（boa_engine 0.19 已有 Promise 支持）
- XMLHttpRequest 基础支持
- 这样 SPA 中的 API 调用能被拦截和执行

**验证站**: `jsonplaceholder.typicode.com` (纯 API SPA)

#### P2-4: setTimeout / setInterval (3-5 天)
- boa_engine 有 microtask 但无 macrotask 调度
- 实现简易事件循环：收集所有 setTimeout → 排序 → 顺序执行
- 配合 fetch 桥接，能跑 "加载后延迟请求" 的 SPA

### 🥈 Tier 2: 强烈建议 — 扩大可爬范围

#### P2-5: Cloudflare / 基础 WAF 绕过 (2-3 周)
- 检测 Cloudflare challenge 页面 (403 + CF 特征)
- 集成 undetected-chromedriver 模式 or 指纹拼图
- Turnstile 验证码处理（需 JS 执行 + 指纹配合）
- 这是 "能不能爬大部分站" 的关键

**验证站**: `www.cloudflare.com`, `nowsecure.nl`

#### P2-6: CSS 选择器增强 (1 周)
- 当前缺少: `:nth-child()`, `:not()`, `+` (相邻兄弟), `~` (通用兄弟)
- 属性选择器增强: `[attr*=val]`, `[attr^=val]`, `[attr$=val]`
- 通配符 `*`
- 这对爬虫价值极高（大部分站用复杂选择器定位数据）

#### P2-7: 重试与错误恢复 (3-5 天)
- 指数退避重试
- 429 Rate Limit 自动等待
- 超时分层配置 (connect / read / total)
- 日志结构化 (tracing + JSON)

### 🥉 Tier 3: 锦上添花 — 产品化

#### P2-8: 并发爬取调度 (1-2 周)
- 内置并发 URL 队列 + 速率限制
- 同域自动节流
- Depth-first / Breadth-first 爬取模式
- 类似 Scrapy 的 pipeline 概念

#### P2-9: 数据提取 DSL (1 周)
- 声明式数据提取：指定 selector → 自动提取文本/属性/href
- 输出 JSON / CSV
- 类似 parsel + item.py 的方式

#### P2-10: 存储与缓存增强 (1 周)
- SQLite 存储已有基础，完善之
- HTTP 缓存 (ETag / Last-Modified / Cache-Control)
- 页面快照存储与回放

#### P2-11: 代理池集成 (1 周)
- SOCKS5 代理 (需 tokio-socks)
- 代理轮换策略
- 代理健康检测

---

## 四、目标站实战验证矩阵

| # | 目标站 | 类型 | 验证能力 | Phase 1 能否通过 |
|---|--------|------|----------|-----------------|
| a | example.com | 简单静态 | HTTP + HTML + DOM + 选择器 | ✅ 已通过 |
| b | jsonplaceholder.typicode.com | REST API + SPA | fetch 桥接、Promise | ❌ 无 fetch |
| c | news.ycombinator.com | 基础反爬 (JS 校验) | 现代 JS 语法兼容 | ❌ boa_engine 报错 |
| d | tls.browserleaks.com | TLS 指纹检测 | TLS ClientHello 模拟 | ❌ rustls 泄露 |
| e | www.cloudflare.com | Cloudflare WAF | 全栈指纹 + challenge 处理 | ❌ 完全挡死 |

### 详细验证计划

**b. jsonplaceholder.typicode.com**
- 验证: fetch() 调用 → 拿到 JSON 数据 → 在 DOM 中展示
- 需要: P2-3 fetch 桥接

**c. news.ycombinator.com**
- 验证: 页面 JS 不报错 → 能提取所有帖子标题和链接
- 需要: 替换 boa_engine 或升级版本修复 ES2015+ 语法支持

**d. tls.browserleaks.com**
- 验证: 返回的 TLS 指纹匹配 Chrome 而非 rustls
- 需要: P2-1 TLS 指纹模拟

**e. www.cloudflare.com**
- 验证: 通过 Cloudflare challenge → 拿到真实页面内容
- 需要: P2-1 + P2-2 + P2-5 全部完成

---

## 五、关键技术决策

### 5.1 JS 引擎选型 — boa_engine 还能用吗？

**问题**: boa_engine 0.19 对 ES2015+ 支持不完整 (对象方法简写、可选链、nullish coalescing 都缺)

**选项**:
1. **升级 boa_engine** 到最新版 (0.20+) — 可能修复部分问题
2. **换用 QuickJS via rquickjs** — 更完整的 ES2020 支持，体积小 (C FFI)
3. **换用 Deno_core** — V8 绑定，完整 JS 支持，但体积 +50MB
4. **JS 转译层** — 用 SWC 把现代 JS 转成 ES5 再喂给 boa_engine

**推荐**: 先升级 boa_engine；如果不够，换 rquickjs (QuickJS)。Deno_core 太重。

### 5.2 TLS 库选型

| 库 | TLS 指纹 | 维护状态 | 体积影响 |
|----|----------|---------|---------|
| rustls (当前) | ❌ 独特指纹 | 活跃 | 已有 |
| boring (BoringSSL) | ✅ 可模拟 Chrome | 活跃 | +2MB |
| native-tls (OpenSSL) | ⚠️ 取决于系统 | 活跃 | 不确定 |
| rustls + 指纹定制 | ⚠️ 部分可行 | 实验性 | 已有 |

**推荐**: **boring** — BoringSSL 是 Chrome 用的库，天然支持 Chrome 指纹。

---

## 六、Phase 时间估算

| Phase | 内容 | 时间 | 里程碑 |
|-------|------|------|--------|
| **Phase 2a** (4 周) | TLS 指纹 + HTTP 头对齐 + fetch 桥接 | 4-6 周 | 能爬 80% 的 API 站和基础站 |
| **Phase 2b** (3 周) | setTimeout + JS 引擎升级 + 选择器增强 | 3-4 周 | 能处理大部分 SPA |
| **Phase 2c** (3 周) | Cloudflare 绕过 + 重试机制 | 3-4 周 | 能对抗基础 WAF |
| **Phase 3** (4 周) | 并发调度 + DSL + 存储 + 代理池 | 4 周 | 产品化 |

**总预估**: Phase 2 全部完成约 10-14 周 (2.5-3.5 个月)

---

## 七、开源准备

### 7.1 项目命名

| 候选名 | 优点 | 缺点 |
|--------|------|------|
| **minibrowser** | 直白，和 crate 名一致 | 搜索结果太多 |
| **mb-engine** | 缩写一致 | 太短 |
| **fetchscape** | 有品牌感 | 不直观 |
| **lightcrawl** | 直观 | 和已有的 lightcrawler 冲突 |
| **scrapeling** | 有特色 | 有点怪 |

**推荐**: 保持 **minibrowser**，GitHub org 下叫 `nousresearch/minibrowser`，crate 名 `minibrowser-core` / `minibrowser-cli`。

如果觉得太通用，可以叫 **mb-engine** 或 **microbrowser**。

### 7.2 开源前必须做的

- [ ] **README.md** — 项目介绍、安装、快速上手、架构图、和 Playwright/curl_cffi 对比表
- [ ] **LICENSE** — MIT 已在 Cargo.toml 声明
- [ ] **CHANGELOG.md** — 从 Phase 1 开始
- [ ] **测试套件** — 每个 crate 至少有单元测试
  - mb-html: 解析标准 HTML 文档
  - mb-dom: CSS 选择器匹配
  - mb-js: JS 执行 + console 输出
  - mb-network: 请求构建 (mock)
  - mb-core: 端到端 navigate
- [ ] **CI/CD** — GitHub Actions
  - `cargo test` on Linux/macOS/Windows
  - `cargo clippy` + `cargo fmt --check`
  - 二进制构建 + release
- [ ] **docs.rs 文档** — 每个公共 API 有 doc comment
- [ ] **examples/ 目录** — 基础用法示例

### 7.3 项目结构调整建议

```
minibrowser/
├── crates/
│   ├── minibrowser-core/    (重命名自 mb-core)
│   ├── minibrowser-network/ (重命名自 mb-network)
│   ├── minibrowser-dom/
│   ├── minibrowser-html/
│   ├── minibrowser-js/
│   ├── minibrowser-storage/
│   ├── minibrowser-fingerprint/
│   └── minibrowser-cli/     (二进制)
├── examples/
│   ├── basic_navigate.rs
│   ├── css_selector.rs
│   └── js_eval.rs
├── tests/
│   └── integration/
├── docs/
│   └── architecture.md
├── README.md
├── ROADMAP.md
├── CHANGELOG.md
└── Cargo.toml
```

---

## 八、Phase 2 里程碑检查表

### Milestone 2a: "能爬基础站" (Week 4)
- [ ] TLS 指纹模拟 (Chrome 模式)
- [ ] HTTP 头自动对齐
- [ ] fetch() 桥接可用
- [ ] 能通过 tls.browserleaks.com 验证
- [ ] 能抓取 jsonplaceholder API 数据

### Milestone 2b: "能跑 SPA" (Week 8)
- [ ] setTimeout/setInterval 支持
- [ ] JS 引擎升级 (boa 0.20+ 或 rquickjs)
- [ ] CSS 选择器增强 (:nth-child, :not, +, ~)
- [ ] 能正确提取 news.ycombinator.com 数据

### Milestone 2c: "能打 WAF" (Week 12)
- [ ] Cloudflare challenge 检测
- [ ] 基础反爬对抗 (403 重试、challenge 处理)
- [ ] 指数退避重试
- [ ] 能通过 www.cloudflare.com 基础检查

### Milestone 3: "产品化" (Week 16)
- [ ] 并发爬取调度
- [ ] 数据提取 DSL
- [ ] 代理池集成
- [ ] 完整测试套件 + CI/CD
- [ ] v0.2.0 发布
