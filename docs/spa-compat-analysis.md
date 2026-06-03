# minibrowser SPA 框架兼容性分析

> 分析日期: 2026-06-03
> 分析范围: mb-js crate (rquickjs 0.12 / QuickJS-NG)

## 关键发现: 文档与代码不同步

**docs/js-engine-roadmap-analysis.md 严重过时。** 它基于 boa_engine 0.19 编写，
但实际代码已迁移到 rquickjs (QuickJS-NG)，架构完全不同:

| 维度 | 旧文档描述 (boa_engine) | 实际实现 (rquickjs 0.12) |
|------|------------------------|--------------------------|
| JS 引擎 | boa_engine 0.19 | rquickjs 0.12 (QuickJS-NG) |
| Promise 支持 | ❌ 无 | ✅ 原生支持 |
| async/await | ❌ 无 | ✅ 原生支持 |
| Proxy | 部分支持 | ✅ 完整支持 |
| 微任务队列 | ❌ 无 | ✅ 内置 Job Queue |
| fetch | ❌ 无 | ✅ 同步底层 + Promise 包装 |
| XMLHttpRequest | ❌ 无 | ✅ 同步底层 |
| setTimeout/setInterval | ❌ 无 | ✅ 实现 |
| MutationObserver | ❌ 无 | ✅ 实现 |
| requestAnimationFrame | ❌ 无 | ✅ 实现 |
| getComputedStyle | ❌ 无 | ✅ mock 实现 |
| IndexedDB | ❌ 无 | ✅ mock 实现 |
| WebSocket | ❌ 无 | ✅ 实现 |

## 当前 Web API 覆盖度

### 已实现 (26+ APIs)

**核心运行时:**
- ✅ Promise / async-await (rquickjs 原生)
- ✅ setTimeout / setInterval / clearTimeout / clearInterval
- ✅ queueMicrotask
- ✅ requestAnimationFrame / cancelAnimationFrame
- ✅ structuredClone

**网络:**
- ✅ fetch (同步底层，返回 Promise)
- ✅ XMLHttpRequest (同步底层)
- ✅ WebSocket
- ✅ navigator.sendBeacon stub

**DOM:**
- ✅ document.getElementById / querySelector / querySelectorAll
- ✅ document.createElement / createTextNode / createDocumentFragment / createComment
- ✅ document.createElement / createRange / createEvent
- ✅ Element.appendChild / insertBefore / removeChild / remove / cloneNode
- ✅ Element.getAttribute / setAttribute / hasAttribute / removeAttribute
- ✅ Element.classList (add/remove/contains/toggle)
- ✅ Element.matches / closest / contains
- ✅ Element.innerHTML / textContent / outerHTML (getter+setter)
- ✅ Element.addEventListener / removeEventListener / dispatchEvent
- ✅ Element.parentNode / parentElement / children / childNodes
- ✅ Element.firstChild / lastChild / nextSibling / previousSibling
- ✅ Element.getElementsByTagName / getElementsByClassName
- ✅ Element.getBoundingClientRect / getClientRects (stub)
- ✅ Element.dataset
- ✅ MutationObserver
- ✅ getComputedStyle (mock)

**浏览器 API:**
- ✅ Event / CustomEvent / EventTarget
- ✅ localStorage / sessionStorage (内存)
- ✅ navigator (userAgent, plugins, platform, etc.)
- ✅ screen
- ✅ window.chrome
- ✅ performance.now()
- ✅ history API (stub)
- ✅ matchMedia (stub)
- ✅ atob / btoa
- ✅ URL / URLSearchParams
- ✅ Blob / FormData
- ✅ TextEncoder / TextDecoder
- ✅ crypto.subtle.digest
- ✅ DOMParser
- ✅ IndexedDB (mock)
- ✅ AbortController (stub)
- ✅ CSS.supports (stub)

### SPA 框架初始化的阻断点分析

#### Vue 3 初始化流程

Vue 3 初始化路径:
```
1. createApp(App)
   - 需要 Proxy (✅ rquickjs 原生支持)
   - 需要 Reflect (✅ rquickjs 原生支持)

2. app.mount('#app')
   - 需要 document.querySelector('#app') (✅)
   - 需要 document.createElement() (✅)
   - 需要 Element.appendChild() (✅)
   - 需要 Element.setAttribute() (✅)

3. 响应式系统 (reactivity)
   - new Proxy(target, handler) (✅)
   - WeakMap / WeakSet / WeakRef (⚠️ 需要验证)

4. 渲染 + 调度
   - queueMicrotask (✅)
   - Promise.resolve().then() (✅)
   - requestAnimationFrame (✅)

5. 模板编译
   - 纯 JS 操作 (✅)
```

#### React 18 初始化流程

React 18 初始化路径:
```
1. ReactDOM.createRoot(container)
   - 需要 document.getElementById/querySelector (✅)

2. root.render(<App />)
   - createElement → React.createElement (Babel 已转译)
   - 需要 Symbol / Symbol.iterator (✅ rquickjs 原生)

3. Concurrent Features
   - MessageChannel / postMessage (⚠️ 未实现)
   - requestAnimationFrame (✅)
   - setTimeout (✅)

4. Suspense / lazy
   - Promise (✅)
   - dynamic import (❌ 需要模块系统)

5. 状态更新调度
   - queueMicrotask (✅)
   - Promise (✅)
```

## 根本原因分析: SPA 可能失败的 5 个阻断点

### 阻断点 #1: fetch/XHR 的同步阻塞 (P1 - 高)

**问题**: fetch 和 XMLHttpRequest 底层都是同步阻塞的 HTTP 请求。
虽然 fetch 返回 Promise 且 rquickjs 支持原生 Promise，
但 fetch 内部的 `_native_fetch()` 是一个同步的 Rust NativeFunction 调用。

```javascript
// 当前 fetch 实现
globalThis.fetch = function(url, options) {
    var raw = _native_fetch(method, url, body, headersStr);  // 同步阻塞!
    var result = JSON.parse(raw);
    return new Promise(function(resolve, reject) {
        resolve(response);  // 立即 resolve
    });
};
```

**影响**:
- 如果 SPA 脚本在 fetch Promise resolve 之前执行了其他同步逻辑，
  微任务队列可能来不及处理
- 多个并发 fetch 无法并行执行（串行阻塞）
- SPA 框架的 SSR 数据获取模式（async setup）可能在数据返回前就尝试渲染

**风险等级**: 中。因为 rquickjs 的微任务队列会在 eval 结束后 drain，
大多数简单的 fetch().then() 模式应该能工作。

### 阻断点 #2: 微任务 drain 的时机限制 (P1 - 高)

**问题**: 当前的微任务 drain 只在以下时机发生:
1. `JsEngine::eval()` 调用结束后
2. timer callback 执行后

但 SPA 框架的典型模式是:
```javascript
// Vue 3 的调度器
Promise.resolve().then(() => {
    // 更新 DOM
    element.textContent = newValue;
    // 又产生新的微任务
    Promise.resolve().then(() => { /* ... */ });
});
```

如果嵌套微任务深度超过 100 层（当前 `ctx.execute_pending_job()` 循环上限），
后续微任务不会被执行。

**影响**: Vue 3 的响应式更新调度、React 18 的并发更新可能被截断。

### 阻断点 #3: DOM 元素原型链不完整 (P2 - 中)

**问题**: `document.createElement()` 创建的元素使用 `__dom_element_proto__` 
作为原型，但这个原型是纯 JS 对象，没有正确的 DOM 接口继承链。

```javascript
// 当前:
createdElement.__proto__ === __dom_element_proto__  // OK
// 但 __dom_element_proto__ 不是 HTMLElement.prototype 的实例

// Vue/React 期望:
createdElement instanceof HTMLElement  // → false ❌
```

**影响**:
- Vue 3 的 `isHTMLTag()` 检查可能失败
- React 的 DOM 节点类型检查可能异常
- 某些库使用 `instanceof Node` / `instanceof Element` 进行类型守卫

**修复方案**: 将 `__dom_element_proto__` 的原型链设置为:
```
__dom_element_proto__ → HTMLElement.prototype → Element.prototype → Node.prototype
```

### 阻断点 #4: 缺少的 DOM API (P2 - 中)

以下 API 在 SPA 框架中经常使用但未实现:

| API | 使用场景 | 影响 |
|-----|---------|------|
| `Node.prototype.contains()` | Vue 的 portal/teleport | 已在 Element 上实现 |
| `document.createComment()` | Vue 的 v-if 注释节点 | ✅ 已实现 |
| `Element.prototype.removeAttribute()` | 通用 | ✅ 已实现 |
| `document.implementation` | DOMParser 创建 | 未验证 |
| `document.createDocumentFragment()` | Vue 的 fragment | ✅ 已实现 |
| `Range` API | Vue 的 SSR hydration | 部分实现 |
| `TreeWalker` / `NodeIterator` | 某些框架 | ❌ 未实现 |
| `document.adoptNode()` / `importNode()` | 跨文档节点 | ❌ 未实现 |
| `CSSStyleSheet` / `document.styleSheets` | CSS-in-JS | ❌ 未实现 |
| `document.head` getter | 脚本注入 | ✅ 已实现 |

### 阻断点 #5: 模块系统 / dynamic import (P3 - 低)

**问题**: 现代 SPA 使用 ES modules 或打包工具的 dynamic import:
```javascript
import('./chunk-abc.js').then(module => { ... })
```

当前 JS 引擎不支持:
- `import()` 表达式
- `import.meta`
- ES module 的 `<script type="module">`

**影响**: 
- Vite 构建的 Vue 3 项目使用大量 dynamic import
- React lazy loading 使用 React.lazy(() => import('./Component'))
- 但打包后的 SPA 通常只有一个 bundle.js，不依赖 dynamic import

**风险等级**: 低。大多数 SPA 在生产环境使用打包后的单文件。

## 优先级排序

### P0 - 必须修复 (SPA 核心路径)

1. **DOM 元素原型链修复**
   - 将 __dom_element_proto__ 挂载到 HTMLElement.prototype 上
   - 确保 `createElement() instanceof HTMLElement === true`
   - 工作量: 1-2 天
   - 文件: `crates/mb-js/src/dom_bridge.rs`

2. **微任务 drain 深度增加**
   - 将 execute_pending_job 循环从 100 提升到 10000
   - 或使用 while 循环 + 全局超时
   - 工作量: 0.5 天
   - 文件: `crates/mb-js/src/lib.rs`

### P1 - 应该修复 (SPA 常见模式)

3. **fetch 并发支持**
   - 当前 fetch 是同步阻塞的，无法并发
   - 方案: 改为异步实现，使用 tokio::spawn + channel 回传结果
   - 工作量: 3-5 天
   - 文件: `crates/mb-js/src/xhr.rs`

4. **MessageChannel / postMessage**
   - React 18 调度器使用 MessageChannel 进行任务切片
   - 可以用 setTimeout(fn, 0) 模拟
   - 工作量: 1 天

5. **CSSStyleSheet / insertRule**
   - CSS-in-JS 库 (styled-components, emotion) 需要
   - 可以用 mock 实现
   - 工作量: 1-2 天

### P2 - 按需修复

6. **TreeWalker / NodeIterator**
7. **document.adoptNode / importNode**
8. **dynamic import() 支持** (需要模块系统)

## 验证建议

要确认 B站排行榜具体失败原因，建议:
1. 在 minibrowser 中 fetch B站排行榜页面 HTML
2. 提取所有 `<script>` 标签的 src 和内容
3. 逐个执行脚本，捕获 JS 错误
4. 分析第一个报错点，定位缺失的 API

```bash
cargo run --release -- navigate --eval "document.scripts.length" "https://www.bilibili.com/v/popular/rank/all"
```
