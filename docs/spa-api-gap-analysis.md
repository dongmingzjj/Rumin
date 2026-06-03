# SPA Framework API Gap Analysis

> Generated: 2026-06-03
> Purpose: Analyze Vue 3 / React 18 core dependencies vs minibrowser's current API coverage

## 一、当前已实现 API 清单 (42 项)

### JS 引擎基础 (QuickJS-NG 原生支持)
| API | 说明 |
|-----|------|
| Proxy / Reflect | ES2015, Vue3 响应式核心, QuickJS 原生 ✅ |
| Symbol / Symbol.for / Symbol.iterator | QuickJS 原生 ✅ |
| Map / Set / WeakMap / WeakSet | QuickJS 原生 ✅ |
| Promise / async-await | QuickJS 原生 ✅ |
| Iterator / Generator | QuickJS 原生 ✅ |
| class 语法 | QuickJS 原生 ✅ |
| 可选链 ?. / 空值合并 ?? | QuickJS 原生 ✅ |

### 已实现的 Web API Polyfill
| # | API | 文件 | 完整度 |
|---|-----|------|--------|
| 1 | console (log/error/warn) | web_apis.rs | ⚠️ 基础 |
| 2 | navigator (UA/platform/language/plugins等) | web_apis.rs | ✅ 完整 |
| 3 | screen (width/height/orientation) | web_apis.rs | ✅ 完整 |
| 4 | window.chrome (runtime/loadTimes/csi) | web_apis.rs | ✅ 完整 |
| 5 | performance (now/timing/timeOrigin) | web_apis.rs | ✅ 完整 |
| 6 | location (href/protocol/host/pathname等) | web_apis.rs | ✅ 完整 |
| 7 | atob / btoa | web_apis.rs | ✅ 完整 |
| 8 | matchMedia | web_apis.rs | ✅ 完整 |
| 9 | localStorage | web_apis.rs | ⚠️ 内存 |
| 10 | sessionStorage | web_apis.rs | ⚠️ 内存 |
| 11 | setTimeout / clearTimeout | timers.rs | ✅ 完整 |
| 12 | setInterval / clearInterval | timers.rs | ✅ 完整 |
| 13 | requestAnimationFrame / cancelAnimationFrame | animation.rs | ✅ 完整 |
| 14 | Event / CustomEvent / EventTarget | event.rs | ✅ 完整 |
| 15 | MutationObserver | mutation_observer.rs | ✅ 完整 |
| 16 | IntersectionObserver | intersection_observer.rs | ⚠️ 简化版 |
| 17 | getComputedStyle | computed_style.rs | ⚠️ 默认值 |
| 18 | Canvas 2D Context | canvas.rs | ⚠️ Mock |
| 19 | IndexedDB | indexed_db.rs | ⚠️ 内存Mock |
| 20 | Intl (DateTimeFormat/NumberFormat等) | intl.rs | ⚠️ Mock |
| 21 | TextEncoder / TextDecoder | text_encoding.rs | ✅ 完整 |
| 22 | URL / URLSearchParams | url_api.rs | ✅ 完整 |
| 23 | crypto.getRandomValues / subtle.digest | crypto_api.rs | ✅ SHA-256 |
| 24 | DOMParser | dom_parser.rs | ✅ 完整 |
| 25 | Blob / FormData | blob_formdata.rs | ⚠️ 基础 |
| 26 | fetch() | xhr.rs | ⚠️ 同步包装 |
| 27 | XMLHttpRequest | xhr.rs | ⚠️ 同步实现 |
| 28 | WebSocket | websocket.rs | ✅ 真实网络 |
| 29 | history (pushState/replaceState) | web_apis.rs | ⚠️ 空函数 |
| 30 | AbortController | web_apis.rs | ⚠️ Stub |
| 31 | Request / Response / Headers | web_apis.rs | ⚠️ Stub |
| 32 | Image 构造函数 | web_apis.rs | ⚠️ 基础 |
| 33 | CSS.supports | web_apis.rs | ⚠️ 返回false |
| 34 | getSelection | web_apis.rs | ⚠️ Stub |
| 35 | navigator.sendBeacon | web_apis.rs | ⚠️ Stub |
| 36 | navigator.webdriver = false | anti_detect.rs | ✅ |
| 37 | navigator.permissions.query | anti_detect.rs | ✅ |
| 38 | queueMicrotask | web_apis.rs (P0) | ✅ 完整 |
| 39 | structuredClone | web_apis.rs (P0) | ⚠️ JSON序列化 |
| 40 | String.prototype.replaceAll | web_apis.rs (P0) | ✅ 完整 |
| 41 | Array.prototype.at | web_apis.rs (P0) | ✅ 完整 |
| 42 | Node / Element / HTMLElement 构造函数 | web_apis.rs | ⚠️ 基础 |

### DOM Bridge (via bind_dom)
document.getElementById, querySelector, querySelectorAll, createElement,
createTextNode, createDocumentFragment, appendChild, removeChild, insertBefore,
cloneNode, setAttribute, getAttribute, removeAttribute, hasAttribute, classList,
textContent, innerHTML, outerHTML, style, className, tagName, id,
parentNode, parentElement, children, childNodes, firstChild, lastChild,
nextSibling, previousSibling 等

---

## 二、Vue 3 核心依赖分析

### 2.1 Vue 3 Reactivity (必须)
| 依赖 | minibrowser 状态 | 备注 |
|------|------------------|------|
| Proxy / Reflect | ✅ QuickJS 原生 | Vue3 响应式核心, 不可替代 |
| Symbol / Symbol.iterator | ✅ QuickJS 原生 | 用于迭代器协议 |
| Map / Set / WeakMap / WeakSet | ✅ QuickJS 原生 | effect 依赖追踪 |
| Promise | ✅ QuickJS 原生 | 异步调度 |

### 2.2 Vue 3 Runtime (必须)
| 依赖 | minibrowser 状态 | 备注 |
|------|------------------|------|
| queueMicrotask | ✅ 已实现 | Vue3 调度器核心 |
| requestAnimationFrame | ✅ 已实现 | Vue3 DOM 更新批处理 |
| MutationObserver | ✅ 已实现 | Vue3 DOM 更新检测 |
| document.createComment | ❌ **缺失** | v-if / Teleport / Suspense 创建注释节点 |
| Comment 节点支持 | ⚠️ 部分 | DOM bridge 有 Comment 类型但 JS 侧可能不完整 |
| performance.now() | ✅ 已实现 | Vue3 开发模式性能追踪 |

### 2.3 Vue 3 Template Compiler (必须)
| 依赖 | minibrowser 状态 | 备注 |
|------|------------------|------|
| DOMParser | ✅ 已实现 | 模板解析 |
| document.createElement | ✅ 已实现 | |
| Map / Set | ✅ QuickJS 原生 | AST 处理 |

### 2.4 Vue 3 Component Libraries 常用
| 依赖 | minibrowser 状态 | 备注 |
|------|------------------|------|
| ResizeObserver | ❌ **缺失** | Element Plus / Vuetify 等响应式组件 |
| IntersectionObserver | ✅ 已实现(简化) | 虚拟滚动 / 懒加载 |
| CSS.supports | ⚠️ 返回 false | 可能影响 CSS-in-JS |
| HTMLElement.offset*/client* | ❌ **缺失** | Popover / Tooltip 定位计算 |
| getComputedStyle | ⚠️ 默认值 | 需要返回 element-specific 值 |
| HTMLElement.dataset | ❌ **缺失** | data-* 属性便捷访问 |

---

## 三、React 18 核心依赖分析

### 3.1 React 18 Scheduler (关键!)
| 依赖 | minibrowser 状态 | 备注 |
|------|------------------|------|
| **MessageChannel** | ❌ **缺失 (最高优先)** | React 18 scheduler 首选调度机制 |
| **MessagePort** | ❌ **缺失** | MessageChannel 的一部分 |
| requestIdleCallback | ❌ **缺失** | React scheduler 降级方案 |
| requestAnimationFrame | ✅ 已实现 | React 用于动画帧调度 |
| performance.now() | ✅ 已实现 | Scheduler 时间切片 |

### 3.2 React 18 Core (必须)
| 依赖 | minibrowser 状态 | 备注 |
|------|------------------|------|
| Proxy / Reflect | ✅ QuickJS 原生 | React DevTools |
| Symbol / Symbol.for | ✅ QuickJS 原生 | Element 标识 |
| Symbol.iterator | ✅ QuickJS 原生 | Fragment 迭代 |
| Map / Set / WeakMap / WeakSet | ✅ QuickJS 原生 | Fiber 树 |
| queueMicrotask | ✅ 已实现 | 微任务调度 |
| Promise | ✅ QuickJS 原生 | Suspense 核心 |
| Object.assign | ✅ QuickJS 原生 | 属性合并 |
| Object.is | ✅ QuickJS 原生 | 状态比较 |
| Error.prepareStackTrace | ❌ **缺失** | React DevMode 错误堆栈增强 |
| globalThis | ✅ 已实现 | |
| performance.now() | ✅ 已实现 | |

### 3.3 React 18 Concurrent Features
| 依赖 | minibrowser 状态 | 备注 |
|------|------------------|------|
| MessageChannel | ❌ **缺失** | 并发渲染调度核心 |
| AbortController | ⚠️ Stub | Suspense + 数据获取取消 |
| fetch (with AbortSignal) | ⚠️ 同步包装 | RSC 数据获取 |

### 3.4 React DOM (必须)
| 依赖 | minibrowser 状态 | 备注 |
|------|------------------|------|
| document.createElement | ✅ 已实现 | |
| document.createTextNode | ✅ 已实现 | |
| document.createComment | ❌ **缺失** | React 注释占位节点 |
| appendChild / removeChild | ✅ 已实现 | |
| setAttribute / removeAttribute | ✅ 已实现 | |
| style 属性 | ✅ 已实现 | |
| event delegation (addEventListener) | ✅ 已实现 | React 事件委托 |
| CustomEvent | ✅ 已实现 | |
| EventTarget | ✅ 已实现 | |

---

## 四、缺失 API 优先级排序

### 🔴 P0 — 框架无法运行 (阻断性缺失)

| # | API | 影响框架 | 实现难度 | 说明 |
|---|-----|---------|---------|------|
| 1 | **MessageChannel / MessagePort** | React 18 | 中 | React scheduler 首选调度机制。无此 API，React 18 完全无法调度任务。需要基于 postMessage 或 setTimeout 降级。 |
| 2 | **document.createComment** | Vue 3 / React 18 | 低 | Vue 的 v-if/Teleport/Suspense、React 的注释占位都依赖此 API。DOM bridge 需要暴露 createElement 的 comment 变体。 |
| 3 | **requestIdleCallback** | React 18 | 低 | React scheduler 降级方案。setTimeout 包装即可。 |

### 🟡 P1 — 框架部分功能受损

| # | API | 影响框架 | 实现难度 | 说明 |
|---|-----|---------|---------|------|
| 4 | **ResizeObserver** | Vue 3 / React 18 | 中 | 响应式组件库 (Element Plus, Ant Design) 必需。可简化为触发一次后 disconnect。 |
| 5 | **HTMLElement.offset*/client* 属性** | Vue 3 / React 18 | 中 | Popover/Tooltip/Modal 定位。需返回合理默认值 (0 或 element 估算尺寸)。 |
| 6 | **HTMLElement.dataset** | Vue 3 / React 18 | 低 | data-* 属性代理，纯 getter 从 attributes 中提取。 |
| 7 | **structuredClone 深拷贝增强** | Vue 3 / React 18 | 中 | 当前 JSON.parse(JSON.stringify()) 丢失函数/Date/RegExp/循环引用。 |
| 8 | **AbortController 增强** | React 18 | 低 | 当前 Stub 不触发真实取消。需要与 fetch 联动。 |

### 🟢 P2 — 增强兼容性

| # | API | 影响框架 | 实现难度 | 说明 |
|---|-----|---------|---------|------|
| 9 | **console 增强 (debug/info/table/group)** | Vue 3 / React 18 | 低 | 开发模式调试输出 |
| 10 | **BroadcastChannel** | SPA 多标签 | 低 | 跨标签通信 |
| 11 | **CSSStyleSheet / adoptedStyleSheets** | Shadow DOM 组件 | 高 | CSS-in-JS 注入 |
| 12 | **Range / getBoundingClientRect 增强** | 富文本/选择器库 | 高 | 精确布局计算 |
| 13 | **navigator.connection** | 自适应加载 | 低 | 网络状态检测 |
| 14 | **Error.prepareStackTrace** | React DevMode | 低 | 增强错误堆栈 |

---

## 五、实现建议

### 5.1 P0 实现方案

#### MessageChannel / MessagePort
```javascript
// 方案 A: 基于 postMessage (推荐)
globalThis.MessageChannel = function() {
    var self = this;
    var queue = [];
    this.port1 = {
        postMessage: function(msg) {
            setTimeout(function() {
                if (self.port2.onmessage) {
                    self.port2.onmessage({ data: msg });
                }
            }, 0);
        },
        onmessage: null,
        close: function() {}
    };
    this.port2 = {
        postMessage: function(msg) {
            setTimeout(function() {
                if (self.port1.onmessage) {
                    self.port1.onmessage({ data: msg });
                }
            }, 0);
        },
        onmessage: null,
        close: function() {}
    };
};
```
注: React 使用 MessageChannel(port2.onmessage) 作为高优先级任务调度。
用 setTimeout(fn, 0) 模拟是可接受的降级方案。

#### document.createComment
```javascript
// 在 DOM bridge 中添加
// document.createComment(data) -> Comment 节点
// Comment 节点: nodeType=8, nodeName="#comment", data=...
```

#### requestIdleCallback
```javascript
globalThis.requestIdleCallback = function(callback, options) {
    var timeout = (options && options.timeout) || 1;
    return setTimeout(function() {
        callback({
            didTimeout: false,
            timeRemaining: function() { return 10; }
        });
    }, timeout);
};
globalThis.cancelIdleCallback = function(id) { clearTimeout(id); };
```

### 5.2 测试验证矩阵

| 测试场景 | 验证 API | 目标 |
|---------|----------|------|
| Vue 3 最小应用 | createComment + queueMicrotask + Proxy | `<div v-if="show">Hello</div>` 正确渲染 |
| React 18 最小应用 | MessageChannel + requestIdleCallback | `ReactDOM.createRoot(el).render(<App />)` 不报错 |
| Element Plus 组件 | ResizeObserver + offset* | `<el-popover>` 正确定位 |
| Ant Design 组件 | MessageChannel + ResizeObserver | `<a-table>` 响应式布局 |
| 状态管理 (Pinia/Redux) | structuredClone + Proxy | 深拷贝不丢失数据 |

---

## 六、总结

### 已覆盖: 42 项 API
minibrowser 已经实现了大量基础 Web API，涵盖了 Vue 3/React 18 约 70% 的核心依赖。

### 关键缺失: 3 项 P0 API
1. **MessageChannel / MessagePort** — React 18 调度核心，缺失则 React 完全无法运行
2. **document.createComment** — Vue 3/React 18 注释节点，缺失则模板编译失败
3. **requestIdleCallback** — React 18 降级调度，缺失则低优先级任务无法调度

### 建议优先级
实现 P0 的 3 项 API 后，minibrowser 应能运行 Vue 3 和 React 18 的最小应用。
P1 的 5 项 API 可在后续迭代中补充，确保组件库兼容性。
