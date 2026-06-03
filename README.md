# Rumin

一个极简的 Rust 浏览器引擎，专为自动化网页抓取和 SPA 渲染设计。无头运行，内置 JS 执行、WAF 挑战自动处理、请求录制与回放。

## 特性

- **SPA 渲染** — 内嵌 JS 引擎，支持 React/Vue 等单页应用的服务端渲染
- **WAF 自动挑战** — crypto.subtle.digest、location.reload 循环、atob/btoa 等 Web API 全填充，自动通过 36kr 等站点的 WAF 验证
- **26+ Web API polyfill** — fetch、DOMParser、URLSearchParams、Blob、FormData、WebSocket、canvas、indexedDB 等
- **请求录制与回放** — 记录完整的请求/响应，支持离线回放调试
- **代理支持** — HTTP/SOCKS5 代理，适用于需要翻墙访问的场景
- **批量并发** — std::thread::spawn 并发导航多个 URL
- **反检测** — navigator、screen、canvas 指纹模拟，避免被识别为自动化工具

## 已验证站点

| 站点 | 状态 |
|------|------|
| Bilibili | SPA 渲染成功 |
| Boss直聘 | SPA 渲染成功 |
| 拉勾 | SPA 渲染成功 |
| 豆瓣 | SPA 渲染成功 |
| 知乎 | SPA 渲染成功 |
| 掘金 | SPA 渲染成功 |
| 小红书 | SPA 渲染成功 |
| 36氪 | WAF 通过，需处理 CAPTCHA |

## 项目结构

```
crates/
├── mb-core     # 核心引擎：页面导航、JS 执行、DOM 构建
├── mb-network  # HTTP 客户端、代理、请求拦截
├── mb-dom      # DOM 节点树、选择器、序列化
├── mb-html     # HTML 解析器（自定义 parser）
├── mb-js       # JS 引擎绑定、Web API polyfill
└── mb-cli      # 命令行入口
```

## 快速开始

```bash
# 编译
cargo build --release

# 访问页面并提取标题
cargo run --release -- navigate --title "https://example.com"

# 通过 CSS 选择器提取内容
cargo run --release -- navigate --selector "h1" "https://example.com"

# 执行 JS 表达式
cargo run --release -- navigate --eval "document.title" "https://example.com"

# 使用代理
cargo run --release -- --proxy socks5://127.0.0.1:7890 navigate "https://google.com"

# 批量导航（JSON 配置）
cargo run --release -- batch batch.json
```

## 测试

```bash
cargo test              # 运行所有测试（78+ 测试用例）
cargo test -p mb-js     # 仅运行 JS 引擎相关测试
cargo test -p mb-network  # 仅运行网络层测试
```

## 技术栈

- **Rust** (edition 2021)
- **boa_engine** — 纯 Rust JS 引擎
- **reqwest** — HTTP 客户端
- **sha2** — SHA-256 实现（WAF 挑战）
- **scraper** — CSS 选择器
- **clap** — CLI 参数解析

## License

Apache-2.0
