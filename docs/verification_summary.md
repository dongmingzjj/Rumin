# Minibrowser 多站点验证总结

## 验证时间
2026-06-03

## P0 修复内容
1. **queueMicrotask()** — Vue3/React18 必需
2. **structuredClone()** — 状态管理库常用
3. **String.prototype.replaceAll()** — 一行 polyfill
4. **Array.prototype.at()** — 一行 polyfill
5. **cloneNode(true)** — 修复子节点 ID bug
6. **appendChild** — 修复 _createdId 支持
7. **document.body/head** — 缓存引用，支持 === 断言
8. **querySelectorAll** — 修复作用域过滤

## 验证结果

### SSR 站点（可正常工作）

| 站点 | 标题 | JS错误 | 数据提取 | 状态 |
|------|------|--------|----------|------|
| 豆瓣 Top250 | ✅ | 低 | ✅ 250/250 部电影 | 完美 |
| 小红书 | ✅ | 低 | ✅ 125 个链接可提取 | 良好 |
| B站首页 | ✅ | 5个 | - | SSR 可用 |
| 百度 | ✅ | 13个 | - | SSR 可用 |

### SPA 站点（需客户端 JS）

| 站点 | 标题 | JS错误 | 数据提取 | 状态 |
|------|------|--------|----------|------|
| 掘金 | ✅ | 6个(第三方SDK) | ⚠️ 需 API 调用 | SSR+API 混合方案可行 |
| B站排行榜 | ❌ 空 | 5个 | ❌ 纯 SPA 无数据 | 需完整 JS 运行时 |

### WAF 阻断站点

| 站点 | 标题 | JS错误 | 数据提取 | 状态 |
|------|------|--------|----------|------|
| 知乎 | ❌ 无标题 | N/A | ❌ WAF 拦截(40362) | 反爬机制阻断 |
| 36kr | ❌ 无标题 | N/A | ❌ WAF/CAPTCHA | 需处理验证码 |

## SSR + API 混合方案验证

### 掘金文章提取
- **方案**: 直接调用掘金 API，无需 JS 渲染
- **API**: `https://api.juejin.cn/recommend_api/v1/article/recommend_all_feed`
- **结果**: 成功提取 20 篇文章，包含标题、ID、URL
- **脚本**: `/tmp/juejin_extract.sh`

## 关键发现

1. **SSR 站点是甜点** — 豆瓣、小红书、百度、B站首页等有 SSR 的站点工作良好
2. **纯 SPA 站点仍受限** — 掘金文章列表、B站排行榜等需要完整客户端 JS 执行
3. **WAF 是硬阻断** — 知乎的 zse-ck 反爬机制无法绕过
4. **API 直接调用可行** — 对于有公开 API 的站点（如掘金），可以直接调用 API 获取数据

## 下一步建议

1. **扩展 API 直接调用** — 为其他有公开 API 的站点创建提取脚本
2. **探索 CAPTCHA 解决方案** — 对于 36kr 等有验证码的站点
3. **完善 JS 运行时** — 提升纯 SPA 站点的渲染能力

## 文件位置

- 验证脚本: `/tmp/juejin_extract.sh`
- 掘金文章数据: `/tmp/juejin_articles.json`
- 豆瓣电影数据: `/tmp/douban_movies_final.txt`
