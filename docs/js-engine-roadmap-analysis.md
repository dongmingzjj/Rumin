# Minibrowser JS 引擎层技术路线分析

## 当前架构评估

### 已实现
- boa_engine 0.19 作为 JS 运行时（纯 Rust，无 C 依赖）
- DOM 树使用 SlotMap 存储（mb-dom crate）
- JS-DOM 桥接：单向快照序列化（DomTree → JS object literals）
- 基础 Web API：console.log, navigator.userAgent, location.href
- 内联脚本执行（但遇到错误会跳过）

### 关键瓶颈
- 所有 DOM 元素序列化为 JS 字面量对象，修改不回写 Rust DomTree
- 无事件循环，无法处理 setTimeout/Promise
- `document.createElement` 创建的元素只存在于 JS 侧，Rust 端无感知
- boa_engine 的 `set_global` 通过 eval 注入，非原生 API 绑定

---

## 1. 事件循环与异步支持

### 方案 A: 留在 boa_engine + tokio 定时器

**实现策略：**
- 注册 `setTimeout(fn, ms)` 为 NativeFunction，内部用 `tokio::time::sleep` 调度
- 维护一个待执行 callback 队列（VecDeque）
- `eval()` 完成后，drain 队列执行所有到期 callback
- `setInterval` 同理，执行后重新入队

**Promise 策略：**
- boa_engine 0.19 **不支持 Promise**（无 JobQueue 概念）
- 只能用 polyfill 模拟：用回调链手动实现 Promise-like 对象
- 示例：`var __promise_queue__ = []; function fakeThen(fn) { __promise_queue__.push(fn); }`
- 这种方式无法支持 `async/await`，只能处理 `.then()` 链式调用

**工作量估算：**
- setTimeout/setInterval 基础实现：2-3 天
- callback drain loop 集成到 Page::navigate：1 天
- Promise polyfill（有限功能）：3-5 天
- fetch/XHR bridge（依赖 tokio async runtime）：2-3 天
- **总计：约 8-12 天**

**局限性：**
- 不支持 `async/await` 语法（需要原生 Promise 支持）
- 复杂的异步代码（如 axios）会失败
- 事件循环调度粒度粗（只能在 eval 边界执行）

### 方案 B: 切换到 QuickJS-NG + napi 绑定

**QuickJS-NG 优势：**
- 内置 Promise/async-await 完整支持
- 内置 Job Queue（微任务队列）
- 内置 `Date.now()`、`performance.now()` 的基础实现
- 更完整的 ES2023 支持（包括 Intl 基础）
- 体积小（~700KB），嵌入式友好
- Rust 绑定：`rquickjs` crate 成熟

**QuickJS-NG 劣势：**
- 依赖 C 编译器（cross-compilation 复杂度上升）
- GC 是引用计数 + 周期检测，与 boa_engine 的 GC handle 模型不同
- API 风格更底层，DOM 绑定需要更多 unsafe FFI 封装

**工作量估算：**
- 引擎替换 + 基础 API 重新绑定：5-7 天
- 事件循环集成（QuickJS 原生支持 job queue）：2-3 天
- Promise/async-await：0 额外工作（原生支持）
- fetch/XHR bridge：2-3 天
- **总计：约 10-14 天（但功能上限更高）**

### 推荐决策

| 维度 | 方案 A (boa + tokio) | 方案 B (QuickJS-NG) |
|------|---------------------|---------------------|
| 开发量 | 8-12 天 | 10-14 天 |
| async/await | ❌ 不支持 | ✅ 完整支持 |
| Promise | 有限 polyfill | 原生支持 |
| 复杂 SPA 兼容 | 差 | 好 |
| 部署复杂度 | 低（纯 Rust） | 中等（需 C 编译） |
| 长期维护 | 需要 hack 更多 | 更正统 |

**建议：先做方案 A 的最小可用版（setTimeout + 基础 Promise polyfill），如果后续遇到大量 SPA 兼容问题，再迁移方案 B。**

原因：
- 当前目标是反检测爬虫，不需要完美运行所有 SPA
- 大多数反检测检测的是 navigator/plugins/timing，不是 async 能力
- boa_engine 的纯 Rust 优势在 CI/CD 和跨平台编译上有实际价值

---

## 2. JS-DOM 双向绑定

### 当前问题
```
JS 侧: document.createElement('div') → 只存在于 __dom_elements__ 对象
Rust 侧: dom tree 完全不知道这些变化
```

### 推荐方案：NativeFunction + NodeId Handle 映射

**核心设计：**
1. 每个 DOM 节点在 JS 侧不是一个纯对象，而是一个 boa_engine 的 JsObject
2. 对象上绑定 NativeFunction 方法（`getAttribute`, `setAttribute`, `appendChild`, etc.）
3. NativeFunction 内部通过闭包捕获 `&mut DomTree` 引用

**关键挑战：**
- boa_engine 的 NativeFunction 需要 `&mut Context`，但 DomTree 也需要 `&mut`
- Rust 不允许两个可变借用
- **解决方案：** 使用 `Rc<RefCell<DomTree>>` 或共享指针模式

**架构草案：**
```rust
// Page 结构变化
pub struct Page {
    dom: Rc<RefCell<DomTree>>,  // 包装为共享可变引用
    js: JsEngine,
    // ...
}

// JsEngine 变化
pub struct JsEngine {
    context: Context,
    dom_ref: Option<Rc<RefCell<DomTree>>>,  // 持有 DOM 引用
}

// 绑定示例
fn bind_get_element_by_id(engine: &mut JsEngine) {
    let dom = engine.dom_ref.clone();
    engine.context.register_global_callable(
        "__native_getElementById__",
        1,
        move |_, args, ctx| {
            let id = args.get(0)?.to_string(ctx)?;
            let dom = dom.borrow();
            if let Some(node_id) = dom.get_element_by_id(&id) {
                // 返回一个 JsObject，带有 node_id 内嵌
                create_js_element_proxy(ctx, node_id)
            } else {
                Ok(JsValue::null())
            }
        }
    );
}
```

**性能考量：**
- 大 DOM 树（10000+ 节点）：SlotMap 访问是 O(1)，性能不是瓶颈
- GC handle 开销：每个 DOM 节点需要一个 JsObject handle
- 内存：10000 节点 × ~200 bytes ≈ 2MB，可接受
- **真正的问题：** 序列化大数组（如 `querySelectorAll` 返回 1000 个元素）的 GC 压力

**优化策略：**
- 惰性创建：JS 对象只在被访问时才创建，而非一次性序列化全部
- WeakRef：用 WeakHandle 避免阻止 GC
- 节点池：LRU 缓存常用的 JS 对象包装

**工作量估算：**
- 基础架构改造（Rc<RefCell<DomTree>>）：2-3 天
- Element Proxy Object + NativeFunction 绑定：5-7 天
- appendChild/removeChild/insertBefore 回写：2-3 天
- 性能优化（惰性创建）：3-5 天
- **总计：约 12-18 天**

### 替代方案：Mutation Queue 模式

不直接同步，而是：
1. JS 侧操作记录到 mutation queue（`__mutations__.push({op:'appendChild', parent, child})`）
2. eval 完成后，Rust 侧遍历 queue 应用变更
3. 简单但不支持同步 DOM 查询（如 appendChild 后立即 querySelector）

对于爬虫场景，Mutation Queue 可能足够用，工作量约 3-5 天。

---

## 3. Web API 注入优先级

### 按反检测重要性排序：

**P0 - 必须实现（直接影响反检测）：**

1. **navigator.plugins** — 指纹检测的高频指标
   ```js
   navigator.plugins = [
     { name: "Chrome PDF Plugin", filename: "internal-pdf-viewer", description: "Portable Document Format" },
     { name: "Chrome PDF Viewer", filename: "mhjfbmdgcfjbbpaeojofohoefgiehjai", description: "" },
     { name: "Native Client", filename: "internal-nacl-plugin", description: "" }
   ];
   navigator.plugins.length = 3;
   // 需要实现 NamedItem / Item 方法
   ```
   工作量：1-2 天（纯 JS 注入即可）

2. **screen 对象** — 几乎所有指纹库都会检测
   ```js
   var screen = {
     width: 1920, height: 1080,
     availWidth: 1920, availHeight: 1040,
     colorDepth: 24, pixelDepth: 24
   };
   ```
   工作量：0.5 天

3. **window.chrome** — 检测 Headless Chrome 的标准手段
   ```js
   var chrome = {
     app: { isInstalled: false, InstallState: { DISABLED: "disabled", INSTALLED: "installed", NOT_INSTALLED: "not_installed" }, RunningState: { CANNOT_RUN: "cannot_run", READY_TO_RUN: "ready_to_run", RUNNING: "running" } },
     runtime: { OnInstalledReason: {}, OnRestartRequiredReason: {}, PlatformArch: {}, PlatformNaclArch: {}, PlatformOs: {}, RequestUpdateCheckStatus: {} }
   };
   ```
   工作量：1 天

4. **performance.now()** — Timing 指纹检测
   - 需要从 Rust 侧注入真实时间戳
   - 使用 `std::time::Instant::now()` 注入
   - 工作量：0.5-1 天

**P1 - 重要（影响反检测但非致命）：**

5. **Error.stack V8 格式** — 某些检测脚本会检查
   - boa_engine 的 Error.stack 格式与 V8 不同
   - 可以用 JS polyfill 重写 Error.prototype.stack 的 getter
   - 工作量：1-2 天

6. **Intl API** — 语言/时区指纹
   - boa_engine 的 Intl 支持有限
   - 可以用 polyfill 注入基础实现
   - 工作量：2-3 天

7. **WebGL 指纹** — 需要 GPU 上下文，headless 环境无法实现
   - 可以注入预计算的 WebGL 参数
   - 工作量：2-3 天

**P2 - 低优先级：**

8. **AudioContext 指纹** — 注入预计算值即可
9. **Canvas 指纹** — 无法真正渲染，只能注入假值
10. **IndexedDB / localStorage** — 需要持久化存储配合

### 建议立即实现 P0 项（总计 3-4 天）

---

## 4. boa_engine vs QuickJS vs V8

### boa_engine 能力边界

**已验证可行：**
- 基础 ES2020 语法（let/const, arrow functions, destructuring）
- JSON.parse/stringify
- 基础字符串/数组操作
- 同步 eval 执行

**已知不足：**
- ❌ 无 Promise / async-await（致命缺陷）
- ❌ 无事件循环（setTimeout 不可能原生支持）
- ❌ Intl 支持不完整
- ❌ 某些边缘 ES 特性缺失（如 Proxy 的某些用法）
- ❌ 无 WeakRef/FinalizationRegistry
- ❌ 性能不如 QuickJS/V8（对爬虫场景影响不大）

### 引擎切换决策矩阵

| 条件 | 建议 |
|------|------|
| 只需要同步脚本 + 简单 DOM | 留在 boa_engine |
| 需要 async/await + Promise | 切换 QuickJS-NG |
| 需要完美 V8 兼容（如 puppeteer 级别） | 切换 V8（但复杂度极高） |
| 需要 WASM 支持 | QuickJS-NG 或 V8 |
| 需要纯 Rust 编译链 | 留在 boa_engine |

### QuickJS-NG 详细分析

**优势：**
- Fabrice Bellard 编写，代码质量极高
- 支持 ES2023 几乎全部特性
- 内置 Promise/async-await 且行为与 V8 高度一致
- Intl 基础支持内置
- 嵌入式场景成熟（被 manylinux, wasmtime 等项目使用）
- Rust 绑定 `rquickjs` 维护活跃

**劣势：**
- C 依赖（需要 cc crate 或系统 C 编译器）
- GC 模型不同（引用计数 vs tracing GC）
- DOM 绑定的 Rust FFI 代码更复杂
- 交叉编译需要额外配置

### V8 评估

**结论：不建议。**
- v8 crate 依赖巨大（~50MB 编译产物）
- 编译时间长（30+ 分钟）
- API 变化频繁
- 与 minibrowser "轻量" 的定位冲突

---

## 实施路线图（优先级排序）

### Phase 2A: 反检测基础（1-2 周）
1. ✅ navigator.plugins 完整注入（1-2 天）
2. ✅ screen 对象注入（0.5 天）
3. ✅ window.chrome 注入（1 天）
4. ✅ performance.now() 实现（1 天）
5. ✅ setTimeout/setInterval 最小实现 + tokio（3 天）
6. ✅ 基础 Promise polyfill（2-3 天）

### Phase 2B: DOM 增强（2-3 周）
7. JS-DOM 双向绑定基础（Mutation Queue 模式，5 天）
8. fetch/XHR bridge 到 mb-network（3-5 天）
9. Error.stack V8 格式兼容（1-2 天）

### Phase 2C: 引擎评估（按需）
10. 如果 Phase 2A-2B 遇到 boa_engine 上限问题
11. 评估迁移到 QuickJS-NG 的成本
12. 只在确有必要时执行迁移

---

## 具体技术建议

### setTimeout 最小实现代码框架

```rust
// 在 JsEngine 中添加
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

struct TimerEntry {
    deadline: Instant,
    callback_id: u64,
    interval: Option<Duration>,
}

pub struct JsEngine {
    context: Context,
    timers: BinaryHeap<TimerEntry>,  // 最小堆按 deadline 排序
    next_timer_id: u64,
}

// 注册 setTimeout 为 NativeFunction
fn register_set_timeout(engine: &mut JsEngine) {
    // NativeFunction: 接收 callback + delay
    // 创建 TimerEntry，存入 heap
    // 返回 timer_id
}

// 在 eval 后调用
fn drain_timers(&mut self) {
    let now = Instant::now();
    while let Some(entry) = self.timers.peek() {
        if entry.deadline > now { break; }
        let entry = self.timers.pop().unwrap();
        // 执行对应的 callback
        self.context.eval(format!("__timer_callbacks__[{}]()", entry.callback_id));
        // 如果是 interval，重新入队
        if let Some(interval) = entry.interval {
            self.timers.push(TimerEntry {
                deadline: now + interval,
                callback_id: entry.callback_id,
                interval: Some(interval),
            });
        }
    }
}
```

### navigator.plugins 注入代码

```js
// 注入到 setup_navigator() 中
(function() {
    var plugins = [];
    plugins[0] = {
        name: "Chrome PDF Plugin",
        filename: "internal-pdf-viewer",
        description: "Portable Document Format",
        length: 1,
        0: { type: "application/x-google-chrome-pdf", suffixes: "pdf", description: "Portable Document Format" }
    };
    plugins[1] = {
        name: "Chrome PDF Viewer",
        filename: "mhjfbmdgcfjbbpaeojofohoefgiehjai",
        description: "",
        length: 1,
        0: { type: "application/pdf", suffixes: "pdf", description: "" }
    };
    plugins[2] = {
        name: "Native Client",
        filename: "internal-nacl-plugin",
        description: "",
        length: 2,
        0: { type: "application/x-nacl", suffixes: "", description: "Native Client Executable" },
        1: { type: "application/x-pnacl", suffixes: "", description: "Portable Native Client Executable" }
    };
    plugins.length = 3;
    plugins.item = function(i) { return this[i] || null; };
    plugins.namedItem = function(n) {
        for (var i = 0; i < this.length; i++) { if (this[i].name === n) return this[i]; }
        return null;
    };
    plugins.refresh = function() {};
    navigator.plugins = plugins;

    // mimeTypes
    var mimeTypes = [];
    mimeTypes[0] = { type: "application/x-google-chrome-pdf", suffixes: "pdf", description: "Portable Document Format", enabledPlugin: plugins[0] };
    // ... 类似
    navigator.mimeTypes = mimeTypes;
})();
```
