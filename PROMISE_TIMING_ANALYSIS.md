# Promise/Async 时序问题深度分析与修复方案

## 一、问题根因

### 1.1 核心时序问题

JS Promise 的 `.then()` / `.catch()` / `finally` 回调是**微任务(microtask)**，在当前同步代码执行完后才入队执行。

```
执行流程:
1. ctx.eval(code)        ← 同步执行，捕获返回值
2. execute_pending_job() ← 执行微任务队列（.then 回调）
3. drain_mutations()     ← 应用 DOM 变更
4. drain_timers()        ← 执行 setTimeout 回调
```

问题在于**步骤1已经捕获了返回值**，此时 Promise 的 `.then()` 回调还没执行。

示例:
```js
var r; Promise.resolve(42).then(v => r = v); r;
// eval 返回 undefined（因为 .then 还没执行）
// execute_pending_job 之后 r 才变成 42
```

### 1.2 fetch() 的 Promise 包装问题

fetch() 的实现在 xhr.rs 第 429-461 行:
```js
globalThis.fetch = function(url, options) {
    var result = _native_fetch(method, url, body);  // 同步 HTTP 调用
    return new Promise(function(resolve, reject) {   // 包装为 Promise
        resolve({ok: true, ...});
    });
};
```

虽然 HTTP 请求是同步的，但包装成 Promise 后，`.then()` 回调变成了微任务。

```js
fetch('/api').then(r => r.text()).then(t => { window.result = t; });
// eval 返回最后一个 Promise（未 resolved）
// execute_pending_job 执行链式 .then() 后 window.result 才被设置
```

### 1.3 async/await 模式

```js
async function main() {
    const r = await fetch('/api');
    return r.text();
}
main();  // 返回 Promise（未 resolved）
```

eval 捕获的是 Promise 对象的字符串表示（`"[object Promise]"`），而非实际值。

## 二、rquickjs API 研究结果

### 2.1 Context::with() 的锁机制

```rust
// context/base.rs:121
pub fn with<F, R>(&self, f: F) -> R {
    let guard = self.0.rt().inner.lock();  // 锁定 runtime 内部互斥锁
    let ctx = unsafe { Ctx::new(self) };
    f(ctx)
}
```

`ctx.with()` 会锁定 runtime 的内部互斥锁。因此**不能**在 `ctx.with()` 内部调用 `runtime.execute_pending_job()`（会死锁）。

### 2.2 Ctx::execute_pending_job()

```rust
// context/ctx.rs:404
pub fn execute_pending_job(&self) -> bool {
    let mut ptr = MaybeUninit::<*mut qjs::JSContext>::uninit();
    let rt = unsafe { qjs::JS_GetRuntime(self.ctx.as_ptr()) };
    let res = unsafe { qjs::JS_ExecutePendingJob(rt, ptr.as_mut_ptr()) };
    res != 0
}
```

- **直接调用 C API**，不经过 mutex，可以在 `ctx.with()` 内部安全使用
- 返回 `true` 表示执行了一个 job，`false` 表示队列为空或出错
- **缺点**: 将错误（返回值 -1）和空队列（返回值 0）都映射为 `false`

### 2.3 Runtime::execute_pending_job()

```rust
// runtime/base.rs:181
pub fn execute_pending_job(&self) -> StdResult<bool, JobException> {
    let mut lock = self.inner.lock();  // 需要锁
    lock.execute_pending_job().map_err(...)
}
```

- 返回 `Ok(true)` 执行成功, `Ok(false)` 队列空, `Err(e)` job 抛异常
- **不能**在 `ctx.with()` 内部使用（互斥锁冲突）
- 当前 eval() 在 ctx.with() 外部使用此方法

### 2.4 Runtime::is_job_pending()

```rust
pub fn is_job_pending(&self) -> bool
```

检查是否有 job 待执行。需要锁，不能在 ctx.with() 内使用。

### 2.5 Promise 相关 API

```rust
// Value 方法（由 sub_types! 宏生成）
Value::as_promise() -> Option<&Promise<'js>>   // 尝试转为 Promise
Value::into_promise() -> Option<Promise<'js>>  // 消耗 Value 转为 Promise

// Promise 方法
Promise::state() -> PromiseState               // Pending / Resolved / Rejected
Promise::result<T>() -> Option<Result<T>>      // None=Pending, Some(Ok)=Resolved, Some(Err)=Rejected
Promise::finish<T>() -> Result<T>              // 循环 drain 直到 resolved，WouldBlock 表示卡住

// Ctx 方法
Ctx::has_exception() -> bool                   // 检查是否有待处理异常
Ctx::catch() -> Value<'js>                     // 捕获并清除异常
```

**注意**: `Value::as_promise()` 使用 `type_of().interpretable_as(Type::Promise)` 检测。Promise 是 Object 的子类型。

## 三、当前代码分析

### 3.1 eval() 方法 (lib.rs:118-151)

```rust
pub fn eval(&mut self, code: &str) -> Result<String> {
    // 1. 在 ctx.with() 中执行代码，立即捕获返回值
    let result_str = self.context.with(|ctx| {
        let val: Value = ctx.eval(code)?;
        Ok(js_value_to_string(&val))  // ← 此时 Promise 还未 resolved
    })?;

    // 2. 在 ctx.with() 外部 drain 微任务
    loop {
        match self.runtime.execute_pending_job() {
            Ok(true) => { ... }
            Ok(false) => break,
            Err(e) => { break; }
        }
    }

    // 3. drain mutations
    // 4. drain timers
    Ok(result_str)  // ← 返回的是步骤1的值，不是 Promise resolved 后的值
}
```

**问题**: 步骤1和步骤2之间有时序断裂。微任务在步骤2才执行，但返回值在步骤1就确定了。

### 3.2 fetch() (xhr.rs:429-461)

fetch() 设计是合理的（同步 HTTP + Promise 包装），但配合 eval() 的时序问题导致：
- `fetch().then(cb)` 的 cb 在 eval 返回后才执行
- `await fetch()` 的结果在 eval 返回后才可用

### 3.3 timers (timers.rs:142-161)

drain_and_execute_timers() 执行 timer 回调，但**不 drain 微任务**。如果 timer 回调创建了 Promise，其 .then() 不会执行。

## 四、修复方案

### 方案: 在 ctx.with() 内部 drain 微任务 + Promise 自动解包

核心思路：利用 `ctx.execute_pending_job()` 在 `ctx.with()` 内部 drain 微任务队列，在返回值捕获之前完成所有 Promise 解析。

#### 修改1: lib.rs — 重写 eval() 方法

```rust
/// Evaluate JavaScript code and return the result as a string.
///
/// If the return value is a Promise, drains microtasks until the Promise
/// settles, then returns the resolved value.
pub fn eval(&mut self, code: &str) -> Result<String> {
    let result_str = self.context.with(|ctx| -> rquickjs::Result<String> {
        let val: Value = ctx.eval(code)?;

        // Phase 1: If the return value is a Promise, drain microtasks
        // until it settles, then return the resolved value.
        if let Some(promise) = val.as_promise() {
            let max_iterations = 1000;
            for _ in 0..max_iterations {
                match promise.state() {
                    rquickjs::value::PromiseState::Pending => {
                        if !ctx.execute_pending_job() {
                            // Queue empty, promise won't resolve
                            break;
                        }
                    }
                    _ => break, // Settled
                }
            }

            // Try to get the settled value
            match promise.result::<Value>() {
                Some(Ok(resolved_val)) => {
                    // Successfully resolved — return the resolved value
                    // But first, drain any NEW microtasks from .then() chains
                    for _ in 0..100 {
                        if !ctx.execute_pending_job() { break; }
                    }
                    return Ok(js_value_to_string(&resolved_val));
                }
                Some(Err(_e)) => {
                    // Promise rejected — try to get error info
                    let err_val = ctx.catch();
                    let err_str = js_value_to_string(&err_val);
                    // Still drain remaining microtasks
                    for _ in 0..100 {
                        if !ctx.execute_pending_job() { break; }
                    }
                    return Ok(format!("Error: {}", err_str));
                }
                None => {
                    // Still pending after drain — return Promise representation
                    // Continue to Phase 2 for remaining microtasks
                }
            }
        }

        // Phase 2: Drain all remaining microtasks (side effects)
        // This handles patterns like:
        //   fetch('/api').then(r => { window.data = r; }); "done";
        for _ in 0..100 {
            if !ctx.execute_pending_job() { break; }
        }

        Ok(js_value_to_string(&val))
    }).map_err(|e| anyhow!("JS evaluation error: {:?}", e))?;

    // Drain any DOM mutations that were queued during eval + microtasks
    if let Err(e) = self.drain_js_mutations() {
        tracing::warn!("Failed to drain JS mutations: {}", e);
    }

    // Drain and execute pending timer callbacks
    // Timer callbacks may create new microtasks, so we loop
    for _round in 0..3 {
        let timer_count = self.drain_and_execute_timers_count()?;
        if timer_count == 0 { break; }

        // After timers, drain microtasks again (timer callbacks may create Promises)
        self.drain_microtasks_outside_ctx();
        if let Err(e) = self.drain_js_mutations() {
            tracing::warn!("Failed to drain JS mutations: {}", e);
        }
    }

    Ok(result_str)
}

/// Drain microtasks using runtime.execute_pending_job() (outside ctx.with())
fn drain_microtasks_outside_ctx(&mut self) {
    let mut rounds = 0;
    loop {
        match self.runtime.execute_pending_job() {
            Ok(true) => { rounds += 1; }
            Ok(false) => break,
            Err(e) => {
                tracing::warn!("Promise job error: {:?}", e);
                break;
            }
        }
        if rounds > 100 { break; }
    }
}

/// Execute timer callbacks and return the count (for loop control)
fn drain_and_execute_timers_count(&mut self) -> Result<u32> {
    let code = r#"
    (function() {
        if (typeof __executeAllTimerCallbacks__ === 'undefined') return 0;
        return __executeAllTimerCallbacks__();
    })()
    "#;

    let count = self.context.with(|ctx| -> rquickjs::Result<u32> {
        let result: Value = ctx.eval(code)?;
        Ok(result.as_float().unwrap_or(0.0) as u32)
    }).map_err(|e| anyhow!("Failed to execute timer callbacks: {:?}", e))?;

    Ok(count)
}
```

#### 修改2: timers.rs — 增强 drain_and_execute_timers()

在每个 timer 执行后也 drain 微任务：

```rust
/// Drain and execute pending timer callbacks, up to 5 rounds.
/// Also drains microtasks after each round.
pub fn drain_and_execute_timers(&mut self) -> Result<()> {
    for _round in 0..5u32 {
        let code = r#"
        (function() {
            if (typeof __executeAllTimerCallbacks__ === 'undefined') return 0;
            return __executeAllTimerCallbacks__();
        })()
        "#;

        let count = self.context.with(|ctx| -> rquickjs::Result<u32> {
            let result: Value = ctx.eval(code)?;
            // Drain microtasks inside ctx.with() (timer callbacks may create Promises)
            for _ in 0..50 {
                if !ctx.execute_pending_job() { break; }
            }
            Ok(result.as_float().unwrap_or(0.0) as u32)
        }).map_err(|e| anyhow!("Failed to execute timer callbacks: {:?}", e))?;

        if count == 0 {
            break;
        }
    }
    Ok(())
}
```

#### 修改3: xhr.rs — fetch() 可选改为同步返回（不推荐，会破坏 await）

实际上 fetch() 的 Promise 包装是正确的设计。配合修改1的 eval()，fetch() 可以正常工作：

```js
// 场景1: .then() 链
fetch('/api').then(r => r.text()).then(t => { window.result = t; });
// eval 返回值可能不重要（关注副作用）
// 微任务会在 eval 内部被 drain

// 场景2: async/await
async function main() { const r = await fetch('/api'); return r.text(); }
let p = main();
// eval 检测到 p 是 Promise，drain 微任务直到 resolved，返回 resolved 值

// 场景3: 直接 await（需要调用方使用 eval_promise）
// eval 会自动检测 Promise 返回值并解包
```

#### 修改4: 新增 eval_raw() 方法（不 drain，返回原始值）

有些场景需要获取 Promise 对象本身（而非 resolved 值），新增：

```rust
/// Evaluate JS code and return the raw result without draining microtasks.
/// Returns the Value as-is, even if it's a Promise.
pub fn eval_raw(&mut self, code: &str) -> Result<String> {
    self.context.with(|ctx| -> rquickjs::Result<String> {
        let val: Value = ctx.eval(code)?;
        Ok(js_value_to_string(&val))
    }).map_err(|e| anyhow!("JS evaluation error: {:?}", e))
}
```

## 五、关键注意事项

### 5.1 ctx.execute_pending_job() 的错误吞没问题

`Ctx::execute_pending_job()` 将 job 错误和空队列都返回 `false`。为了不丢失错误信息，在 drain 循环后检查异常：

```rust
// 在 drain 循环结束后
if ctx.has_exception() {
    let err = ctx.catch();
    tracing::warn!("Microtask error: {:?}", js_value_to_string(&err));
}
```

### 5.2 无限循环防护

微任务可能产生新的微任务（如 Promise 链很长）。必须有上限：
- Promise 等待: 最多 1000 轮
- 通用 drain: 最多 100 轮
- Timer + microtask 循环: 最多 3 轮

### 5.3 setTimeout(fn, 0) 的处理

`setTimeout(fn, 0)` 创建的是宏任务（通过 timer 机制），不是微任务。eval() 中的微任务 drain 不会处理它。当前的 drain_and_execute_timers() 在 eval 末尾调用，可以处理。

但注意：timer 回调中创建的 Promise（如 `setTimeout(() => { fetch().then(...) }, 0)`）需要额外的微任务 drain。修改2和修改1的循环处理了这种情况。

### 5.4 破坏性变更评估

修改 eval() 的返回值行为可能影响依赖当前行为的代码：
- 如果代码 eval 一个 Promise 并期望得到 "[object Promise]" 字符串 → 会变成 resolved 值
- 新增 eval_raw() 提供旧行为的替代

## 六、测试用例

```rust
#[test]
fn test_promise_resolve_value() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("Promise.resolve(42)").unwrap();
    assert_eq!(result, "42", "Promise.resolve should return the resolved value");
}

#[test]
fn test_promise_then_value() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("Promise.resolve(42).then(v => v * 2)").unwrap();
    assert_eq!(result, "84", "Promise.then should resolve to the chained value");
}

#[test]
fn test_promise_side_effect() {
    let mut engine = JsEngine::new_with_defaults();
    engine.eval("var r; Promise.resolve(42).then(v => { r = v; });").unwrap();
    let r = engine.eval("r").unwrap();
    assert_eq!(r, "42", "Promise.then side effects should be applied");
}

#[test]
fn test_promise_chained_then() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval(
        "Promise.resolve(1).then(v => v + 1).then(v => v + 1)"
    ).unwrap();
    assert_eq!(result, "3", "Chained .then() should resolve to final value");
}

#[test]
fn test_async_function() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval(
        "(async function() { return 42; })()"
    ).unwrap();
    assert_eq!(result, "42", "async function returning value should be unwrapped");
}

#[test]
fn test_promise_rejection() {
    let mut engine = JsEngine::new_with_defaults();
    let result = engine.eval("Promise.reject('error')");
    // Should return error info
    assert!(result.is_err() || result.unwrap().contains("error"));
}
```
