# Rumin Minibrowser — WAF 对抗模块架构方案

## 一、现状分析

### 已有基础
- `mb-network` 已使用 `wreq` + `wreq-util` 做 TLS 指纹模拟（Chrome136/120, Firefox136, Safari18）
- `mb-js/stealth.js` 已实现 navigator.webdriver=false、chrome 对象注入、RTCPeerConnection 禁用
- `mb-js/anti_detect.js` 已有基础反检测

### 核心问题
当前架构存在以下缺陷导致 WAF 拦截：
1. TLS/HTTP2 指纹虽然用了 wreq，但缺乏**动态轮换**和**一致性校验**
2. HTTP 头顺序未完全匹配 Chrome（reqwest/hyper 默认按字母序）
3. 缺少请求时序模拟（无随机延迟、无资源加载顺序模拟）
4. 无代理池管理，IP 容易被封
5. 无 CAPTCHA 检测与自动处理
6. TCP/IP 层指纹（TTL、窗口大小、MSS）未考虑

---

## 二、核心调研结论

### 2.1 TLS 指纹对抗

**关键发现：`wreq`（crates.io 上为 `newwreq`）是最佳选择**

| 方案 | 说明 | 推荐度 |
|------|------|--------|
| `wreq` + `wreq-util` | 已在用，支持 100+ 浏览器 profile，BoringSSL 精细控制 | ⭐⭐⭐⭐⭐ |
| `reqwest-impersonate` | wreq 的前身，功能类似但已不再维护 | ⭐⭐ |
| `chromimic` | Chrome/OkHttp 模拟，较新 | ⭐⭐⭐ |
| `clawser-fetch` | 用 Chromium 网络栈，过于重量级 | ⭐ |
| `ja3` / `ja3-rustls` | 仅计算 JA3 hash，不控制实际 TLS | ⭐ |

**wreq 支持的 TLS 精细控制（已验证源码）：**
- 协议版本：TLS 1.0 ~ 1.3
- ALPN：HTTP/1.1, HTTP/2, HTTP/3
- ALPS：Application-Layer Protocol Settings
- 密码套件：完整 Chrome cipher list
- 签名算法：ecdsa_secp256r1_sha256 等
- 曲线：X25519, P-256, P-384, X25519MLKEM768
- GREASE ECH
- 扩展排列（permute_extensions）
- Session Ticket / PSK
- 证书压缩（Brotli）

**wreq-util 已覆盖的浏览器版本：**
Chrome 100~148, Edge 101~148, Firefox, Safari, Opera, OkHttp

### 2.2 HTTP 头顺序对抗

**关键发现：wreq 专为此问题设计**

- 标准 `reqwest`/`hyper` 的 `http` 库会将 header 名转为小写且不保留顺序
- wreq 的 `OrigHeaderMap` 保留原始 header 大小写和顺序
- wreq 的 `header_initializer` 宏精确匹配 Chrome 各版本的 header 顺序：
  ```
  Chrome 顺序: sec-ch-ua → sec-ch-ua-mobile → sec-ch-ua-platform
               → upgrade-insecure-requests → user-agent
               → accept → sec-fetch-site → sec-fetch-mode → sec-fetch-user
               → sec-fetch-dest → accept-encoding → accept-language → priority
  ```

**结论：使用 wreq 的 Emulation 系统即可自动匹配正确 header 顺序。**

### 2.3 HTTP/2 指纹对抗

**关键发现：wreq 通过 `Http2Options` 精细控制 SETTINGS 帧**

wreq-util 中 Chrome HTTP/2 配置（已验证源码）：
```rust
Http2Options::builder()
    .initial_window_size(6291456)          // 6MB
    .initial_connection_window_size(15728640) // 15MB
    .max_header_list_size(262144)          // 256KB
    .header_table_size(65536)              // 64KB
    .max_concurrent_streams(1000)
    .headers_stream_dependency(StreamDependency::new(StreamId::zero(), 219, true))
    .headers_pseudo_order(PseudoOrder([
        Method, Authority, Scheme, Path
    ]))
    .settings_order(SettingsOrder([
        HeaderTableSize, EnablePush, MaxConcurrentStreams,
        InitialWindowSize, MaxFrameSize, MaxHeaderListSize,
        EnableConnectProtocol, NoRfc7540Priorities
    ]))
    .build()
```

**这完全覆盖了 Akamai HTTP/2 指纹检测。**

### 2.4 行为模拟可行性

- **鼠标/键盘事件**：在无 WebView 的架构下不可行（没有 DOM 事件循环）
- **替代方案**：通过请求时序 + Cookie/Referer 链来模拟人类行为
- **可行的行为模拟**：
  - 请求间随机延迟（50ms~3000ms 高斯分布）
  - 资源加载顺序模拟（HTML → CSS → JS → 图片）
  - Cookie 持久化（已有 CookieJar）
  - Referer 链追踪

### 2.5 代理/CAPTCHA 对接

- **代理**：wreq 原生支持 HTTP/SOCKS5 代理（已有 ProxyConfig）
- **CAPTCHA**：需要对接外部服务（2Captcha/Anti-Captcha/CapSolver）
- **Cloudflare Turnstile**：`zendriver-cloudflare` crate 可参考

---

## 三、模块架构设计

### 3.1 设计原则
1. **独立 crate**：`crates/mb-waf`，不侵入 mb-core/mb-network
2. **Trait 抽象**：通过 trait 与核心引擎解耦
3. **可插拔**：每个对抗维度独立，可单独启用/禁用
4. **配置驱动**：通过 TOML/YAML 配置，运行时可热更新

### 3.2 文件结构

```
crates/mb-waf/
├── Cargo.toml
└── src/
    ├── lib.rs                    # 模块入口，导出 WafBypass trait
    ├── config.rs                 # WafConfig 配置结构体
    ├── error.rs                  # 错误类型
    │
    ├── fingerprint/              # 指纹管理层
    │   ├── mod.rs
    │   ├── rotation.rs           # 指纹轮换策略（轮询/随机/一致性）
    │   ├── validation.rs         # 指纹一致性校验（JA3/JA4 在线检测）
    │   └── profiles.rs           # 自定义 profile 扩展
    │
    ├── headers/                  # HTTP 头对抗
    │   ├── mod.rs
    │   ├── order.rs              # 头顺序管理（委托给 wreq Emulation）
    │   └── injection.rs          # 动态 header 注入（Accept-Language 随机化等）
    │
    ├── timing/                   # 请求时序模拟
    │   ├── mod.rs
    │   ├── delay.rs              # 随机延迟策略
    │   ├── sequence.rs           # 资源加载顺序模拟
    │   └── human.rs              # 人类行为模式（打字速度、阅读时间）
    │
    ├── proxy/                    # 代理管理
    │   ├── mod.rs
    │   ├── pool.rs               # 代理池（健康检查、自动轮换）
    │   ├── provider.rs           # 代理源接口（API/文件/数据库）
    │   └── balancer.rs           # 负载均衡策略
    │
    ├── captcha/                  # CAPTCHA 处理
    │   ├── mod.rs
    │   ├── detector.rs           # CAPTCHA 类型检测（Turnstile/hCaptcha/reCAPTCHA）
    │   ├── solver.rs             # 打码服务抽象 trait
    │   ├── providers/            # 打码服务实现
    │   │   ├── mod.rs
    │   │   ├── twocaptcha.rs     # 2Captcha
    │   │   ├── anticaptcha.rs    # Anti-Captcha
    │   │   └── capsolver.rs      # CapSolver
    │   └── cache.rs              # 解答缓存
    │
    ├── waf_detect/               # WAF 类型检测
    │   ├── mod.rs
    │   ├── cloudflare.rs         # Cloudflare 特征
    │   ├── akamai.rs             # Akamai 特征
    │   ├── imperva.rs            # Imperva/Incapsula 特征
    │   └── datadome.rs           # DataDome 特征
    │
    └── middleware/               # Tower 中间件
        ├── mod.rs
        ├── retry.rs              # WAF 触发后智能重试
        └── circuit_breaker.rs    # 熔断器（连续失败时切换策略）
```

### 3.3 核心 Trait 设计

```rust
// lib.rs
pub trait WafBypass: Send + Sync {
    /// 在请求前应用 WAF 对抗策略
    fn prepare_request(&self, req: &mut HttpRequest) -> Result<()>;

    /// 在响应后检查是否被 WAF 拦截
    fn check_response(&self, resp: &HttpResponse) -> WafCheckResult;

    /// 处理 WAF 拦截（重试/换指纹/换代理/打码）
    fn handle_block(&self, context: &BlockContext) -> Result<BlockAction>;
}

pub enum WafCheckResult {
    /// 正常通过
    Pass,
    /// 被 WAF 拦截，附带 WAF 类型和拦截原因
    Blocked { waf_type: WafType, reason: BlockReason },
    /// 需要 CAPTCHA 验证
    CaptchaRequired { captcha_type: CaptchaType },
    /// 需要 JS Challenge
    JsChallenge { challenge_body: String },
}

pub enum BlockAction {
    /// 使用新指纹重试
    RetryWithNewFingerprint,
    /// 使用新代理重试
    RetryWithNewProxy,
    /// 等待后重试
    RetryAfterDelay(Duration),
    /// 提交 CAPTCHA
    SolveCaptcha(CaptchaTask),
    /// 放弃
    Abort,
}

pub enum WafType {
    Cloudflare,
    Akamai,
    Imperva,
    DataDome,
    AwsWaf,
    Custom(String),
}
```

### 3.4 指纹轮换策略

```rust
// fingerprint/rotation.rs
pub trait FingerprintStrategy: Send + Sync {
    fn next_profile(&self) -> EmulationProfile;
    fn report_outcome(&self, profile: &EmulationProfile, success: bool);
}

/// 一致性策略：同一会话始终使用同一指纹
pub struct ConsistentStrategy {
    current: RwLock<Option<EmulationProfile>>,
    fallbacks: Vec<EmulationProfile>,
}

/// 轮询策略：每次请求轮换
pub struct RoundRobinStrategy {
    profiles: Vec<EmulationProfile>,
    index: AtomicUsize,
}

/// 自适应策略：根据成功率动态调整
pub struct AdaptiveStrategy {
    profiles: Vec<(EmulationProfile, Mutex<FingerprintStats>)>,
}

struct FingerprintStats {
    requests: u64,
    successes: u64,
    last_used: Instant,
}
```

### 3.5 代理池管理

```rust
// proxy/pool.rs
pub struct ProxyPool {
    proxies: Vec<ProxyEntry>,
    strategy: ProxyStrategy,
    health_checker: HealthChecker,
}

struct ProxyEntry {
    config: ProxyConfig,
    stats: ProxyStats,
    status: ProxyStatus,
}

pub enum ProxyStrategy {
    /// 轮询
    RoundRobin,
    /// 最少使用
    LeastUsed,
    /// 最快响应
    FastestResponse,
    /// 随机
    Random,
    /// 按地理位置
    ByRegion(String),
}

#[async_trait]
pub trait ProxyProvider: Send + Sync {
    /// 获取代理列表
    async fn fetch_proxies(&self) -> Result<Vec<ProxyConfig>>;
    /// 报告代理不可用
    async fn report_failure(&self, proxy: &ProxyConfig) -> Result<()>;
}
```

### 3.6 CAPTCHA 对接

```rust
// captcha/solver.rs
#[async_trait]
pub trait CaptchaSolver: Send + Sync {
    /// 支持的 CAPTCHA 类型
    fn supported_types(&self) -> Vec<CaptchaType>;

    /// 提交解答任务
    async fn solve(&self, task: CaptchaTask) -> Result<CaptchaSolution>;

    /// 查询任务状态
    async fn get_result(&self, task_id: &str) -> Result<CaptchaStatus>;
}

pub struct CaptchaTask {
    pub captcha_type: CaptchaType,
    pub site_key: String,
    pub page_url: String,
    pub proxy: Option<ProxyConfig>,
    pub additional_data: HashMap<String, String>,
}

pub enum CaptchaType {
    Turnstile,      // Cloudflare Turnstile
    HCaptcha,       // hCaptcha
    ReCaptchaV2,    // reCAPTCHA v2
    ReCaptchaV3,    // reCAPTCHA v3
    GeeTest,        // GeeTest
    Funcaptcha,     // FunCaptcha
    AwsWaf,         // AWS WAF captcha
}
```

---

## 四、与核心引擎的集成方式

### 4.1 依赖关系

```
mb-core
  └── mb-network (HTTP 请求)
        └── mb-waf (WAF 对抗，可选 feature)
              └── wreq + wreq-util (TLS/H2 指纹)
```

### 4.2 Cargo.toml

```toml
# crates/mb-waf/Cargo.toml
[package]
name = "mb-waf"
version = "0.1.0"
edition = "2021"

[features]
default = ["cloudflare", "captcha"]
cloudflare = []
akamai = []
captcha = ["dep:reqwest"]
proxy-rotate = []

[dependencies]
wreq = { version = "6", features = ["cookies"] }
wreq-util = "3"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
tracing = "0.1"
async-trait = "0.1"
rand = "0.8"
url = "2"
reqwest = { version = "0.12", optional = true, features = ["json"] }
```

### 4.3 集成示例

```rust
// 在 mb-network/client.rs 中集成
use mb_waf::{WafMiddleware, WafConfig};

impl HttpClient {
    pub fn builder() -> HttpClientBuilder {
        HttpClientBuilder::new()
    }
}

pub struct HttpClientBuilder {
    emulation: EmulationPreset,
    proxy: ProxyConfig,
    waf_config: Option<WafConfig>,
    // ...
}

impl HttpClientBuilder {
    /// 启用 WAF 对抗（可选）
    pub fn with_waf(mut self, config: WafConfig) -> Self {
        self.waf_config = Some(config);
        self
    }

    pub fn build(self) -> Result<HttpClient> {
        let mut builder = wreq::Client::builder()
            .emulation(self.emulation.to_emulation())
            .cookie_store(true);

        // 应用 WAF 中间件
        if let Some(waf) = self.waf_config {
            let waf_middleware = WafMiddleware::new(waf)?;
            // wreq 支持 Tower middleware
            builder = builder.middleware(waf_middleware);
        }

        // ...
    }
}
```

### 4.4 使用示例

```rust
// 最终用户代码
use mb_waf::{WafConfig, FingerprintStrategy, ProxyPool, CaptchaSolverConfig};

let waf_config = WafConfig::builder()
    // 指纹轮换
    .fingerprint(FingerprintStrategy::Adaptive {
        profiles: vec![
            EmulationProfile::Chrome136_Windows,
            EmulationProfile::Chrome142_Windows,
            EmulationProfile::Chrome148_MacOS,
        ],
    })
    // 代理池
    .proxy(ProxyPool::new(
        ProxyStrategy::RoundRobin,
        vec![
            ProxyConfig::Http("http://proxy1:8080".into()),
            ProxyConfig::Socks5("socks5://proxy2:1080".into()),
        ],
    ))
    // CAPTCHA 处理
    .captcha(CaptchaSolverConfig {
        provider: "2captcha".into(),
        api_key: "YOUR_API_KEY".into(),
        supported_types: vec![CaptchaType::Turnstile, CaptchaType::HCaptcha],
    })
    // 行为模拟
    .timing(TimingConfig {
        min_delay_ms: 100,
        max_delay_ms: 2000,
        jitter: 0.3,
    })
    .build();

let client = HttpClient::builder()
    .emulation(EmulationPreset::Chrome136)
    .with_waf(waf_config)
    .build()?;

// 正常使用，WAF 对抗自动生效
let response = client.get("https://protected-site.com").await?;
```

---

## 五、实施路线图

### Phase 1：基础指纹加固（1~2 周）
- [ ] 升级 wreq/wreq-util 到最新版本
- [ ] 实现 `FingerprintStrategy`（一致性 + 轮询）
- [ ] 验证 JA3/JA4 在线检测通过（https://tls.peet.ws/api/all）
- [ ] 确认 HTTP/2 Akamai 指纹匹配

### Phase 2：请求行为模拟（1 周）
- [ ] 实现随机延迟中间件
- [ ] 资源加载顺序模拟
- [ ] Referer 链追踪

### Phase 3：代理池（1 周）
- [ ] ProxyPool 实现
- [ ] 健康检查（定时 ping + 可用性测试）
- [ ] 自动剔除不可用代理

### Phase 4：CAPTCHA 处理（1~2 周）
- [ ] WAF 类型检测（基于响应 header/body 特征）
- [ ] 2Captcha/Anti-Captcha 集成
- [ ] Cloudflare Turnstile 自动处理
- [ ] JS Challenge 引擎（简单 JS 执行，不需完整浏览器）

### Phase 5：自适应优化（持续）
- [ ] 指纹成功率统计
- [ ] 自动降级策略（指纹被封 → 换代理 → 换区域）
- [ ] 熔断器（连续失败时暂停并切换策略）

---

## 六、关键决策总结

| 维度 | 方案 | 理由 |
|------|------|------|
| TLS 指纹 | wreq + wreq-util Emulation | 已在用，100+ profile，BoringSSL 精细控制 |
| HTTP 头顺序 | wreq OrigHeaderMap | 专为保留大小写和顺序设计 |
| HTTP/2 指纹 | wreq Http2Options | 精确控制 SETTINGS/PRIORITY/PseudoOrder |
| TCP 指纹 | 暂不处理 | 需要 raw socket，复杂度高，大部分 WAF 不检测 |
| 行为模拟 | 请求时序 + 延迟中间件 | 无 WebView 限制下最可行的方案 |
| 代理管理 | 自建 ProxyPool | 轮换 + 健康检查 |
| CAPTCHA | 外部服务 | 2Captcha/Anti-Captcha API |
| 模块架构 | 独立 crate + Trait 抽象 | 不侵入核心引擎，可选编译 |
